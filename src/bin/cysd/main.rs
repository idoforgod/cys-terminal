//! cysd — CYSJavis 터미널 헤드리스 코어 데몬.
//! UI와 완전 분리: UI가 hang이어도 이 데몬과 소켓 제어 채널은 항상 살아있다 (OOB 회생).
// Windows: 데몬은 콘솔이 없어야 한다. 콘솔 서브시스템으로 두면 GUI(windows_subsystem)가
// cysd.exe 를 띄울 때 Windows가 실제 콘솔을 할당(Win11=Windows Terminal 검은 빈 창)하고,
// 그 상속 콘솔이 ConPTY 유사콘솔 핸드오프를 오염시켜 셸 surface가 즉시 종료된다([surface exited]).
// GUI 앱과 동일하게 릴리스에서 windows subsystem 으로 빌드해 콘솔을 원천 제거한다(디버그는 콘솔 유지).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod alerts;
mod analytics;
mod approval;
mod approval_risk;
mod boot_supervisor;
mod caps;
mod channels;
mod classifier;
mod cost;
mod deadman;
mod delivery;
mod events;
mod governance;
mod handlers;
mod hwmon;
mod recall;
mod schedule;
mod severity;
mod skillrun;
mod state;
mod undo;
mod usage;

use cys::Request;
use handlers::Reply;
use serde_json::json;
use state::Daemon;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

type Stream = Box<dyn AsyncReadWrite>;

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

// ═══ 업데이트 잔해(.prev*) 회수 — 판정·행동 규칙 (P1b · Windows 전용 · 회귀 핀 대상) ═══
//
// 생성원은 설치기 두 곳뿐이다(src-tauri/nsis-hooks.nsh):
//   ① CYS_SWAP_IN_PLACE(:130-141)  — `<bin>.prev.exe` · `.prev2.exe` · `.prev3.exe` 고정 3칸 체인
//   ② unlock-sweep.ps1(:355-371)   — `<원본이름>.prev<rand>` (rand = Get-Random -Maximum 99999)
// 둘 다 "잠긴 파일을 죽이는 대신 이름을 비우는" 무중단 업데이트의 부산물이고, 그 뒷정리는
// **새 cysd 기동**이 진다(nsis-hooks.nsh L6).
//
// ★2026-08-26 T4-6 회귀에서 드러난 종전 설계의 결손:
//   삭제(remove_file)는 **매핑된 PE 이미지에 대해 반드시 실패**한다. 종전 코드는 그 실패를
//   무음 스킵했으므로("다음 기동이 마저 청소한다"), 홀더가 살아있는 한 잔해가 runtime 트리에
//   영구히 남고 스윕의 사후 상태가 비결정적이었다. 이 수리는 **삭제 실패를 무시하지 않고**
//   같은 볼륨 격리함으로 rename 해 완결시킨다 — 매핑된 이미지는 삭제는 거부돼도 **rename 은
//   허용**된다(설치기 unlock-sweep 이 이미 의존하는 Windows 특성이고, 매핑은 경로가 아니라
//   파일 오브젝트에 걸리므로 홀더 프로세스는 아무 영향을 받지 않는다).
//   ⇒ 사후 불변식: "부트 후 설치 트리(격리함 제외)에 잔해 0" 이 홀더 생존 여부와 무관하게 성립.
//
// ★W1(2026-08-29) 잔해 회수 **가드**: 위 규칙 전체는 "정식 파일 `<bin>.exe` 가 살아 있다"는
//   전제 위에서만 안전하다. 설치 도중 전원 차단·설치기 강제종료로 "정식 없음 + .prev 만 존재"가
//   되면, 이름 문법만 보는 스윕이 **유일한 동작본을 지운다**(T3 사고 계열 — CLI 영구 소실).
//   그래서 스윕 전에 가드(guard_canonical_binaries)가 cys·cysd·cys-app 3종의 정식 존재를
//   확인하고, 부재면 삭제 대신 복구(승격)·동결한다 — 규칙의 정의처는 plan_leftover_action(순수).
//   설치기가 배치 직전 단계에 만드는 `<bin>.new.exe`(W4-B)는 is_update_leftover 문법 **밖**이다
//   (설치 중 데몬 재부팅이 신본을 격리하면 안 되므로 의도적) — 가드의 명시 규칙이 전담한다.

/// 삭제 불가(=아직 매핑 중) 잔해를 모아두는 **설치 루트 하위** 격리 디렉토리 이름.
/// `runtime\` **밖**이어야 한다 — 격리의 목적이 runtime 트리를 결정론적으로 비우는 것이다.
/// 설치기 unlock-sweep 은 루트 **최상위 파일**만 보므로(하위 디렉토리 미스캔) 격리본이 다시
/// rename 대상이 되는 일도 없다.
#[cfg(any(windows, test))]
pub(crate) const UPDATE_TRASH_DIR: &str = ".cys-update-trash";

/// 한 부트에서 격리할 수 있는 잔해 상한 — 병리적 트리에서 부트가 길어지지 않게 한다.
#[cfg(any(windows, test))]
pub(crate) const MAX_QUARANTINE_PER_BOOT: usize = 500;

/// 업데이트 잔해 파일명 판정(순수 · 회귀 핀).
///
/// 종전 규칙은 `name.contains(".prev")` 였다 — `notes.preview.png` 같은 **사용자 파일**까지
/// 삭제 대상으로 삼는 과대매칭이다. 청소가 실패하는 결함을 고치면서 **범위를 넓히는 대신
/// 좁힌다**(오너 앵커 ④ — 살아있는 파일을 지우는 방향은 금지). 위 생성원 2종의 명명 규칙만
/// 매칭한다: 마지막 `.prev` 뒤가 (숫자*) 또는 (숫자* + `.exe`) 인 경우.
#[cfg(any(windows, test))]
pub(crate) fn is_update_leftover(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(i) = lower.rfind(".prev") else {
        return false;
    };
    let tail = &lower[i + ".prev".len()..];
    let digits_end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    let rest = &tail[digits_end..];
    rest.is_empty() || rest == ".exe"
}

/// 격리본의 파일명(순수 · 회귀 핀). **결과 자체가 다시 `is_update_leftover` 를 만족해야 한다** —
/// 홀더가 죽은 뒤 다음 부트의 스윕이 같은 규칙으로 집어 삭제하는 것이 회수의 마지막 단계다.
/// `.prev<숫자>` 를 덧붙여 그 계약을 이름으로 강제한다(설치기 unlock-sweep 과 같은 형식).
#[cfg(any(windows, test))]
pub(crate) fn quarantine_file_name(orig: &str, stamp: u64, seq: usize) -> String {
    format!("{orig}.prev{stamp}{seq:03}")
}

// ── W1 부팅 잔해 회수 가드 — 정식(canonical) 부재 시 잔해는 쓰레기가 아니라 복구 재료다 ──
//
// 이 가드는 부팅 시퀀스에서 스윕보다 먼저, 소켓 서빙 **전에** 돈다 — 규칙이 틀리면 전 조직의
// CLI 가 죽는 자리다. 그래서 판정 전량을 순수 함수(plan_leftover_action)에 두고, 파일시스템
// 배선(guard_canonical_binaries)은 실행·동결만 한다(스윕과 같은 순수판정+배선 구조).

/// 가드 대상 3종 — 설치 루트($INSTDIR)에 정식 이름으로 놓이는 우리 바이너리 전부.
/// (runtime\ 밑 이미지는 아카이브 전개의 산물이라 `.new` 승격 개념이 없다 — 가드 범위 밖.)
#[cfg(any(windows, test))]
pub(crate) const GUARDED_BINS: [&str; 3] = ["cys", "cysd", "cys-app"];

/// 복구 재료로 믿을 최소 크기 64KiB — 설치기 배치 검증(nsis-hooks W4-B ④)과 같은 임계다.
/// `.new` 는 파일 **복사**의 산물이라 전원 차단 시 잘린 채 남을 수 있고, 잘린 파일을 정식으로
/// 승격하면 다음 부팅이 "정식 존재"로 오판해 **남은 진짜 재료(.prev)를 지운다** — 승격 실수가
/// 삭제 사고로 전이되는 유일한 경로라서, prev 에도 같은 문턱을 건다(정상 prev 는 rename 산물
/// = 원자적이라 잘릴 수 없으므로, 이 문턱이 거르는 것은 병리적 이력의 쓰레기뿐이다).
#[cfg(any(windows, test))]
pub(crate) const MIN_PROMOTABLE_BYTES: u64 = 64 * 1024;

/// 승격 후보(.prev 계열)의 관측값 — 판정의 입력. 순수 판정을 위해 파일시스템이 아니라 값으로
/// 받는다(Windows 전용 상황 "정식 소실"을 어느 OS 의 단위 테스트로도 박제하기 위함).
#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrevCandidate {
    pub name: String,
    /// epoch 초. 못 읽었으면 0 — '가장 오래됨'으로 접혀 mtime 을 아는 후보가 항상 이긴다(보수적).
    pub mtime: u64,
    pub size: u64,
}

/// 한 바이너리 가족에 대한 이번 부트의 행동 계획(순수 · 회귀 핀).
///
/// ★배선 실행 순서 계약: 삭제는 어떤 갈래에서든 **정식이 실재하게 된 뒤에만** 한다. 승격
///   rename 이 실패하면 계획이 무엇이었든 가족 전체를 동결한다 — "정식 부재인데 지운다"로
///   전이하는 분기가 코드 어디에도 없어야 이 가드가 성립한다.
#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LeftoverPlan {
    /// 정식 존재 — 가족 잔해는 종전 규칙(스윕)이 지운다. `delete_new` 는 문법 밖 `<bin>.new` 의
    /// 명시 처분(정식이 있으니 그 신본은 남을 이유가 없는 배치 중단 흔적이다).
    SweepAll { delete_new: bool },
    /// 정식 부재 — `<bin>.new`(≥64KiB) 를 정식으로 rename 승격. 성공 시 잔여 prev 는 스윕이 정리.
    PromoteNew,
    /// 정식 부재·쓸 만한 `.new` 없음 — `source`(최신 mtime 의 승격 가능 prev)를 정식으로 rename
    /// 승격. `delete_new` = 미달(잘린) `.new` 의 처분 지시 — 단 **승격 성공 후에만**(위 계약).
    PromotePrev { source: String, delete_new: bool },
    /// 정식 부재 + 가족 파일 자체가 하나도 없음 — 지킬 것도 고칠 것도 없다(개발 트리·미설치
    /// 바이너리의 정상 상태). **조용히** 지나간다 — 매 부팅 우는 배너는 이 파일이 없애온 실패다.
    NothingToDo,
    /// 정식 부재 + 재료는 있으나 전부 승격 부적격(미달 크기) — **아무것도 지우지 않는다**.
    /// 가족 전체 무접촉 동결 + loud. 잘린 재료도 보존한다(다음 설치·수동 복구의 단서).
    Hold,
}

/// **판정의 정의처(순수).** 정식 존재 여부 · `.new` 크기 · prev 후보들 → 이번 부트의 행동.
#[cfg(any(windows, test))]
pub(crate) fn plan_leftover_action(
    canonical_exists: bool,
    new_size: Option<u64>,
    prevs: &[PrevCandidate],
) -> LeftoverPlan {
    if canonical_exists {
        return LeftoverPlan::SweepAll { delete_new: new_size.is_some() };
    }
    if new_size.is_some_and(|s| s >= MIN_PROMOTABLE_BYTES) {
        return LeftoverPlan::PromoteNew;
    }
    // 최신 mtime 우선 — 가장 마지막까지 정식 자리에 있었던 본이 최신이다. 동률은 이름으로 깨서
    // 부트 간 순서를 결정론으로 고정한다(read_dir 순서 의존 금지 — 격리함 유계 패스와 같은 규율).
    let best = prevs
        .iter()
        .filter(|p| p.size >= MIN_PROMOTABLE_BYTES)
        .max_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.name.cmp(&b.name)));
    if let Some(p) = best {
        return LeftoverPlan::PromotePrev {
            source: p.name.clone(),
            delete_new: new_size.is_some(),
        };
    }
    if new_size.is_none() && prevs.is_empty() {
        LeftoverPlan::NothingToDo
    } else {
        LeftoverPlan::Hold
    }
}

/// `<bin>` 가족의 잔해인가(순수) — `is_update_leftover` 문법 위에 "누구의 것인가"를 얹는다.
/// 마지막 `.prev` 앞 줄기가 정확히 `<bin>` 또는 `<bin>.<ext>` 인 것만 가족이다:
///   체인 슬롯 `cys.prev.exe`·`cys.prev2.exe`(줄기 cys) · unlock-sweep `cys.exe.prev4213`
///   (줄기 cys.exe) · W4-B tick 슬롯 `cys.prev<틱>.exe`(줄기 cys). 정확 일치라 `cys` 가족이
///   `cys-app.prev.exe`·`cysd.prev.exe` 를 삼키는 접두 오염은 구조적으로 불가능하다.
#[cfg(any(windows, test))]
pub(crate) fn is_family_leftover(name: &str, bin: &str, ext: &str) -> bool {
    if !is_update_leftover(name) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    let Some(i) = lower.rfind(".prev") else {
        return false;
    };
    let stem = &lower[..i];
    let bin_l = bin.to_ascii_lowercase();
    stem == bin_l || stem == format!("{bin_l}.{}", ext.to_ascii_lowercase())
}

/// 가드 계수 — 로그(guard_log_lines)와 테스트 핀의 관측 지점. 동결의 계수·고지 소유자는 스윕이
/// 아니라 가드다(스윕의 hold 스킵은 무계수 — 계수 소유자가 하나여야 정직하다, 격리함 규율과 동형).
#[cfg(any(windows, test))]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct GuardStats {
    /// (정식 이름, 재료 이름) — 정식 부재를 rename 승격으로 복구했다. **항상 loud**.
    pub promoted: Vec<(String, String)>,
    /// (정식 이름, 시도한 재료 이름) — 승격 rename 실패. 가족 전체 무접촉 동결. **항상 loud**.
    pub promote_failed: Vec<(String, String)>,
    /// (정식 이름, 보존한 가족 파일 수) — 정식 부재 + 승격 가능 재료 전무. 무행동. **항상 loud**.
    pub held: Vec<(String, usize)>,
    /// 정식 존재(승격 성공 직후 포함) 하에 지운 `<bin>.new.<ext>` 수 — 문법 밖 명시 규칙의
    /// 유일한 삭제 경로. info(배치 중단 흔적 청소 — 조치 불요).
    pub deleted_new: usize,
}

/// 동결: 가족 잔해 전체를 스윕이 건너뛸 집합에 넣는다(`.new` 는 문법 밖이라 스윕이 원래 못 본다).
#[cfg(any(windows, test))]
fn freeze_family(
    hold: &mut std::collections::HashSet<std::path::PathBuf>,
    dir: &std::path::Path,
    family: &[PrevCandidate],
) {
    for c in family {
        hold.insert(dir.join(&c.name));
    }
}

/// 가드 배선 — 설치 루트 **최상위**만 본다(정식·`.new`·가족 prev 는 전부 루트에 놓인다: 체인
/// 슬롯도 unlock-sweep 도 루트만 만든다). 반환값은 스윕이 건너뛸 동결 경로 집합.
///
/// `remove`·`promote` 를 주입받는 이유는 스윕과 같다: Windows 전용 실패 분기(잠긴 대상의
/// rename 거부 등)를 타 OS 의 단위 테스트에서 결정론으로 재현한다. 참고로 승격 rename 은
/// 매핑 중인(실행 중인) 옛 본이 .prev 를 붙들고 있어도 성공한다 — 매핑은 경로가 아니라 파일
/// 오브젝트에 걸린다(파일 상단 주석의 unlock-sweep 이 의존하는 그 Windows 특성).
#[cfg(any(windows, test))]
pub(crate) fn guard_canonical_binaries(
    dir: &std::path::Path,
    ext: &str,
    remove: &mut dyn FnMut(&std::path::Path) -> bool,
    promote: &mut dyn FnMut(&std::path::Path, &std::path::Path) -> bool,
    stats: &mut GuardStats,
) -> std::collections::HashSet<std::path::PathBuf> {
    let mut hold = std::collections::HashSet::new();
    // 루트 잔해를 한 번만 훑어 두고 가족별로 나눈다(가족당 재스캔 금지 — 부트 경로다).
    // DirEntry::metadata 는 심볼릭링크를 따라가지 않으므로 링크는 is_file 에서 탈락한다
    // (링크 너머의 살아있는 파일이 승격 rename 으로 이동되는 경로 차단 — 오너 앵커 ④).
    let mut root_leftovers: Vec<PrevCandidate> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if !md.is_file() {
                continue; // 디렉토리(격리함 포함)·링크는 가드의 관심 밖
            }
            let p = e.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_update_leftover(name) {
                continue;
            }
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            root_leftovers.push(PrevCandidate {
                name: name.to_string(),
                mtime,
                size: md.len(),
            });
        }
    }
    for bin in GUARDED_BINS {
        let canonical_name = format!("{bin}.{ext}");
        let new_name = format!("{bin}.new.{ext}");
        let canonical = dir.join(&canonical_name);
        let canonical_exists = std::fs::metadata(&canonical).is_ok_and(|m| m.is_file());
        let new_path = dir.join(&new_name);
        let new_size = std::fs::metadata(&new_path)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len());
        let family: Vec<PrevCandidate> = root_leftovers
            .iter()
            .filter(|c| is_family_leftover(&c.name, bin, ext))
            .cloned()
            .collect();
        match plan_leftover_action(canonical_exists, new_size, &family) {
            LeftoverPlan::NothingToDo => {}
            LeftoverPlan::SweepAll { delete_new } => {
                // 정식이 살아 있으니 가족 prev 는 스윕 관할로 되돌린다(무동결). `.new` 만 여기서
                // 지운다. 삭제 실패는 조용히 둔다 — 문법 밖이라 스윕도 못 보는 무해 잔류물이고,
                // 잠겨 있다면 설치기가 진행 중이라는 뜻이라 다투지 않는 쪽이 안전하다(다음 설치의
                // 배치 단계가 어차피 지운다 — W4-B ②).
                if delete_new && remove(&new_path) {
                    stats.deleted_new += 1;
                }
            }
            LeftoverPlan::PromoteNew => {
                if promote(&new_path, &canonical) {
                    stats.promoted.push((canonical_name, new_name));
                    // 잔여 prev 는 정식이 실재하게 됐으므로 스윕이 종전 규칙으로 정리한다(무동결).
                } else {
                    stats.promote_failed.push((canonical_name, new_name));
                    freeze_family(&mut hold, dir, &family);
                }
            }
            LeftoverPlan::PromotePrev { source, delete_new } => {
                let src = dir.join(&source);
                if promote(&src, &canonical) {
                    stats.promoted.push((canonical_name, source));
                    // ★정식이 이제 실재한다 — 그때만 미달 `.new` 를 처분한다(실행 순서 계약).
                    if delete_new && remove(&new_path) {
                        stats.deleted_new += 1;
                    }
                } else {
                    stats.promote_failed.push((canonical_name, source));
                    freeze_family(&mut hold, dir, &family);
                }
            }
            LeftoverPlan::Hold => {
                stats
                    .held
                    .push((canonical_name, family.len() + usize::from(new_size.is_some())));
                freeze_family(&mut hold, dir, &family);
            }
        }
    }
    hold
}

/// 가드 결과를 로그 줄로 환원한다(순수 · leftover_log_lines 계열 — 같은 stderr 채널로 흘러간다.
/// 신규 이벤트 타입은 만들지 않는다 — EVENT_CONTRACT 밖 타입은 거부되는 계약).
/// 복구·동결·실패는 전부 **loud** 다: 정식 부재는 "직전 설치가 도중에 죽었다"는 뜻이라 사용자
/// 조치(설치기 재실행)가 실재한다 — "조치 가능한 이상만 ⚠"라는 이 파일의 등급 규율에 부합한다.
#[cfg(any(windows, test))]
pub(crate) fn guard_log_lines(stats: &GuardStats) -> Vec<(LeftoverLog, String)> {
    let mut out = Vec::new();
    for (canonical, source) in &stats.promoted {
        out.push((
            LeftoverLog::Loud,
            format!(
                "[cysd] ⚠ 부팅 복구: 정식 {canonical} 부재 → {source} 를 rename 승격해 재건했다 — \
                 직전 업데이트가 중단된 흔적이다. 설치기를 다시 실행해 최신본으로 마무리하라"
            ),
        ));
    }
    for (canonical, source) in &stats.promote_failed {
        out.push((
            LeftoverLog::Loud,
            format!(
                "[cysd] ⚠ 정식 {canonical} 부재·복구 승격 실패({source} rename 거부) — 복구 재료 \
                 보존을 위해 가족 잔해를 무접촉 동결했다. 다음 기동이 재시도한다"
            ),
        ));
    }
    for (canonical, kept) in &stats.held {
        out.push((
            LeftoverLog::Loud,
            format!(
                "[cysd] ⚠ 정식 {canonical} 부재 + 승격 가능한 복구 재료 전무 — 아무것도 지우지 \
                 않는다(가족 파일 {kept}개 무접촉 보존). 설치기를 다시 실행해 복구하라"
            ),
        ));
    }
    if stats.deleted_new > 0 {
        out.push((
            LeftoverLog::Info,
            format!(
                "[cysd] update leftovers: 정식 존재 확인 후 배치 중단 흔적 .new 신본 {}건 삭제",
                stats.deleted_new
            ),
        ));
    }
    out
}

/// 스윕 계수 — 로그(침묵 금지)와 테스트 핀의 관측 지점.
#[cfg(any(windows, test))]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SweepStats {
    /// 삭제 성공(홀더 없음).
    pub removed: usize,
    /// 삭제 실패 → 격리함으로 rename 성공(홀더 생존 · runtime 트리에서는 사라짐).
    pub quarantined: usize,
    /// 삭제도 격리도 실패 — **살아있는 설치 트리에 잔해가 그대로 남는 유일한 경우**.
    /// 격리함 **안**의 미회수분은 여기에 섞이지 않는다(그건 `TrashBoundStats::remaining`).
    /// 이 값이 0 이 아니면 봉쇄 자체가 실패한 것이므로 **무조건 loud**.
    pub stuck: usize,
    /// 격리는 됐는데 **격리 시각(체류시계)을 심지 못한** 항목 수 — `set_quarantine_stamp` 실패.
    /// 봉쇄는 성공했고 그 항목은 나이 축에서 '신선'으로 접히므로 지금 할 조치가 없다 = **info**.
    /// 침묵하지는 않는다: 나이 승격이 그만큼 늦어지는 사실은 로그에 남아야 사후 진단이 된다.
    pub stamp_failed: usize,
}

/// 격리 시각을 **격리본 자신에게 심는다** — 유계의 나이 축이 재는 것은 파일 *내용*의 나이가
/// 아니라 **격리함 체류시간**이기 때문이다.
///
/// ★없으면 나이 축의 입력이 통째로 틀린다: 격리는 `fs::rename` 이라 원본 mtime 이 그대로
///   보존되는데, 격리 대상의 대종인 runtime PE 이미지는 업스트림 아카이브(PortableGit·Python
///   embeddable 등)에서 풀린 것이라 **수개월 전 mtime** 을 갖는다. 그 값을 나이로 읽으면 격리되는
///   순간 이미 14일 상한을 넘겨 있어 **업데이트 직후 첫 부팅부터** ⚠ 가 뜬다 — 이 수리가 없애려던
///   오탐 배너 클래스 그 자체다(CI 실측: 체류 ~7분인데 aged 8건).
///
/// ★같은 값으로 다시 부르면 **멱등 no-op** 이다. `bound_update_trash` 가 이 성질을 이용해
///   "지금 읽은 mtime 이 우리가 심은 스탬프인가"를 값 변경 없이 되묻는다(그쪽 나이 축 주석 참조).
///
/// Windows 에서 접근권을 `FILE_WRITE_ATTRIBUTES` 로 **좁혀** 여는 것이 load-bearing 이다:
/// 격리본은 아직 홀더가 이미지로 매핑 중인 파일이라 `GENERIC_WRITE` 로는 공유 위반으로 열리지
/// 않는다(로더는 `FILE_SHARE_READ|FILE_SHARE_DELETE` 로 연다). 시각 기록은 데이터 접근이 아니라
/// **속성** 접근이라 공유 모드 검사에 걸리지 않으므로, 잠긴 이미지에도 스탬프가 박힌다.
#[cfg(any(windows, test))]
fn set_quarantine_stamp(path: &std::path::Path, secs: u64) -> bool {
    let Some(t) = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs)) else {
        return false;
    };
    let times = std::fs::FileTimes::new().set_modified(t);
    open_for_stamp(path)
        .and_then(|f| f.set_times(times))
        .is_ok()
}

/// 시각 기록 전용 open — 데이터 쓰기 권한을 요구하지 않는다(위 주석의 공유 위반 회피).
#[cfg(windows)]
fn open_for_stamp(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
    const FILE_SHARE_ALL: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004; // READ|WRITE|DELETE
    std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_ALL)
        .open(path)
}

/// 타 OS(=단위 테스트 레인)에는 속성 전용 접근권이 없다 — 쓰기 열기로 같은 능력을 대표한다.
#[cfg(all(not(windows), test))]
fn open_for_stamp(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).open(path)
}

/// 설치 트리를 재귀 순회하며 업데이트 잔해를 회수한다(삭제 → 실패 시 격리).
///
/// `remove`·`relocate` 를 주입받는 이유는 Windows 전용 실패 분기(=매핑된 이미지 삭제 거부)를
/// 다른 OS 의 단위 테스트에서 결정론으로 재현하기 위함이다(실기기 재현 불가 경로의 박제).
/// 디렉토리는 삭제 대상이 아니다(설치기는 파일만 rename 한다) — 재귀만 한다.
///
/// ★격리함(`trash`)은 **통째로 건너뛴다** — 소유자가 하나여야 계수가 정직해진다.
///   격리함 안의 회수(=삭제 재시도)·유계·승격은 전부 `bound_update_trash` 가 진다.
///   종전에는 스윕이 격리함 안에서도 삭제를 시도하고 실패분을 `stuck` 에 섞었는데, 그 탓에
///   "봉쇄 실패(살아있는 트리에 잔해 잔존)"와 "봉쇄 성공 후 정상 대기"가 같은 계수로 합쳐져
///   같은 ⚠ 경고를 냈다 — 조치할 것이 없는데 부팅마다 울리는 오탐 배너 클래스의 재발이다.
///
/// ★`hold` 는 W1 가드가 동결한 복구 재료(정식 부재 가족의 잔해)의 경로 집합 — **무접촉으로
///   건너뛴다**. 계수도 남기지 않는다: 동결의 계수·고지 소유자는 가드(GuardStats)다(위와 같은
///   소유자 단일화 규율). 가드가 없던 종전 계약은 빈 집합을 넘기는 것과 동치다.
#[cfg(any(windows, test))]
pub(crate) fn sweep_update_leftovers(
    dir: &std::path::Path,
    depth: u8,
    trash: &std::path::Path,
    stamp: u64,
    hold: &std::collections::HashSet<std::path::PathBuf>,
    remove: &mut dyn FnMut(&std::path::Path) -> bool,
    relocate: &mut dyn FnMut(&std::path::Path, &std::path::Path) -> bool,
    stats: &mut SweepStats,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // 권한·경로 문제로 못 읽는 서브트리는 종전과 동일하게 통째 skip
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // 격리함 서브트리는 재귀조차 하지 않는다 — 재격리(무한 이동)·이중 삭제 시도 원천 차단.
            if p == trash {
                continue;
            }
            sweep_update_leftovers(&p, depth - 1, trash, stamp, hold, remove, relocate, stats);
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_update_leftover(name) {
            continue;
        }
        if hold.contains(&p) {
            // W1 가드가 동결한 복구 재료(정식 부재 가족) — 이번 부트는 무접촉. 계수도 없다:
            // 동결의 계수·고지는 가드(GuardStats.held)가 이미 졌다(소유자 단일화).
            continue;
        }
        if remove(&p) {
            stats.removed += 1;
            continue;
        }
        if stats.quarantined >= MAX_QUARANTINE_PER_BOOT {
            stats.stuck += 1;
            continue;
        }
        let dest = trash.join(quarantine_file_name(name, stamp, stats.quarantined));
        if relocate(&p, &dest) {
            stats.quarantined += 1;
            // ★격리 시각을 격리본에 심는다 — 나이 축의 입력을 '내용 mtime'에서 **체류시간**으로
            //   바꾸는 지점이다(rename 은 원본 mtime 을 보존하므로 심지 않으면 남의 시각을 읽는다).
            //   실패는 **신선한 쪽으로** 접는다: 등급은 info 이고, 판정 쪽 접힘은
            //   `bound_update_trash` 의 나이 축이 진다(스탬프 실패가 조기 경고를 만드는 방향 금지).
            if !set_quarantine_stamp(&dest, stamp) {
                stats.stamp_failed += 1;
            }
        } else {
            stats.stuck += 1;
        }
    }
}

// ── 격리함 유계(bound) 정책 ────────────────────────────────────────────────────
//
// 격리는 runtime 트리를 결정론적으로 비우지만, **홀더가 끝내 죽지 않는 세션**에서 업데이트가
// 반복되면 매 세대의 잔해가 격리함에 새로 쌓인다(같은 이름이 아니라 매번 새 파일 오브젝트다).
// 회수의 유일한 기전은 "홀더가 죽은 뒤의 삭제"이고 그건 우리가 앞당길 수 없으므로, 유계 장치는
// ①회수 가능한 것은 매 부트 전량 회수하고 ②회수 불가분을 **계산에서 빼지 않고** 그대로 세어
// ③상한을 넘는 순간 사람에게 소리내어 알리는 것으로 성립한다. 상한 초과를 조용히 삼키면
// 무한 성장이 은폐된다 — 그것이 이 장치가 막는 실패다.
//
// ★안전 경계(오너 앵커 ④): 이 장치는 **우리가 만든 격리함 한 디렉토리의, 우리가 붙인 이름
//   규칙에 맞는 정규 파일**만 만진다. 재귀하지 않고(격리는 항상 격리함 루트에 평평하게 놓는다),
//   디렉토리·심볼릭링크는 건드리지 않으며, 격리함 밖으로는 어떤 삭제도 넓히지 않는다.

/// **나이 상한 14일.** 재는 축은 파일 *내용*의 나이가 아니라 **격리함 체류시간**이다
/// (입력은 격리 순간 `set_quarantine_stamp` 이 심는 시각 — 그 함수 주석이 왜인지를 진다).
/// 격리본이 남아있다는 것은 그 파일 오브젝트를 매핑한 홀더가 아직 살아있다는
/// 뜻이다(홀더가 죽으면 다음 부트의 회수가 즉시 지운다). 데스크톱 세션이 재부팅 없이 2주를
/// 넘겨 같은 이미지를 붙들고 있는 것은 정상 범위 밖이므로, 14일을 넘긴 격리본은 "홀더 문제가
/// 아닌 다른 원인(권한·ACL·백신 잠금)" 신호로 보고 승격 사유로 삼는다.
#[cfg(any(windows, test))]
pub(crate) const TRASH_MAX_AGE_SECS: u64 = 14 * 24 * 60 * 60;

/// **개수 상한 64.** 한 업데이트 세대가 남기는 잔해는 루트 체인 3칸 + unlock-sweep 이 훑는
/// runtime 이미지 몇 개 규모다(T4-6 실측 9개). 64 는 그런 세대가 일곱 번 누적되도록 **한 번도**
/// 회수되지 않은 상태 — "정상 대기 중"이라는 설명이 더는 성립하지 않는 지점이다.
/// ※ 이 축이 재는 것은 세대 수가 아니라 **현재 점유**다. 한 번의 병리적 업데이트가 단숨에
///   64 를 넘겨도 발화하는데, 그것도 정당한 신호다(잠긴 이미지 64개 이상은 그 자체로 이상).
///   홀더가 죽으면 다음 부트의 회수로 0 에 수렴하며 경고도 함께 사라진다 — 자기해소형 경고다.
#[cfg(any(windows, test))]
pub(crate) const TRASH_MAX_ENTRIES: usize = 64;

/// **총 바이트 상한 512 MiB.** 격리 대상의 대종은 runtime PE 이미지(msys-2.0.dll·python313.dll·
/// bash.exe 등 수 MB 단위)다. 개수 상한과 **독립**으로 두는 이유는 소수의 거대 파일이 개수
/// 상한을 우회하는 경로를 막기 위함이고, 512 MiB 는 사용자가 디스크 압박을 체감하기 시작하는
/// 규모다.
#[cfg(any(windows, test))]
pub(crate) const TRASH_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// **한 부트의 회수 시도 예산 2000건.** 병리적으로 커진 격리함에서 부트가 길어지지 않게 한다.
/// 예산은 **오래된 것부터** 쓰고(정렬이 여기서 load-bearing 이다), 예산 밖 항목도 잔존 계수·
/// 바이트에는 **그대로 포함**한다 — 안 세면 무한 성장이 숨는다.
/// 한 부트의 격리 상한(`MAX_QUARANTINE_PER_BOOT` = 500)보다 크게 잡아, 직전 부트가 최대로
/// 격리한 뒤라도 다음 부트가 그 전량에 회수를 시도할 수 있게 한다(예산이 병목이 되지 않는다).
#[cfg(any(windows, test))]
pub(crate) const TRASH_MAX_RECLAIM_PER_BOOT: usize = 2_000;

/// 격리함 유계 패스의 계수 — 로그 등급 판정과 테스트 핀의 관측 지점.
#[cfg(any(windows, test))]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TrashBoundStats {
    /// 삭제 성공 = 회수 완료(홀더 사망). 격리함이 줄어드는 **유일한** 경로.
    pub reclaimed: usize,
    /// 패스 종료 후 격리함에 남은 항목 수 — 회수 실패분·예산 밖 항목을 **전부 포함**한다.
    pub remaining: usize,
    /// 남은 항목의 총 바이트.
    pub remaining_bytes: u64,
    /// 나이 상한(`TRASH_MAX_AGE_SECS`) 초과인데 회수도 못한 항목 수 — 승격 사유.
    pub aged_stuck: usize,
    /// 회수 시도 예산(`TRASH_MAX_RECLAIM_PER_BOOT`)을 넘겨 이번 부트에 손대지 못한 항목 수.
    pub deferred: usize,
}

#[cfg(any(windows, test))]
impl TrashBoundStats {
    /// 평시(=정상 대기) 대 이상(=사람이 봐야 함)의 판정. **이 함수 하나가 경고 등급의 정의처다.**
    ///
    /// 격리함 안의 미회수분은 그 자체로는 이상이 아니다 — 봉쇄가 성공했고 홀더가 죽기를 기다리는
    /// 설계된 상태이며, 사용자가 취할 조치가 없다. 조치할 것이 없는데 부팅마다 ⚠ 를 띄우면
    /// 경고의 신호가치가 죽는다. 그래서 승격은 **유계가 깨졌을 때만** 한다:
    ///   ① 개수 상한 초과 ② 총 바이트 상한 초과 ③ 나이 상한 초과 미회수분 존재.
    /// 셋 다 실측 가능한 축이고, 셋 중 하나라도 참이면 "대기"로는 설명되지 않는 상태다.
    pub fn over_bound(&self) -> bool {
        self.remaining > TRASH_MAX_ENTRIES
            || self.remaining_bytes > TRASH_MAX_BYTES
            || self.aged_stuck > 0
    }
}

/// 격리함을 회수·유계한다. 스윕 **뒤**에 돌아야 이번 부트에 새로 격리된 것까지 계산에 든다.
///
/// 계약:
/// - 대상은 `trash` **직속**의 정규 파일 중 `is_update_leftover` 를 만족하는 이름뿐이다
///   (격리본은 `quarantine_file_name` 계약상 항상 이 규칙을 만족한다 — 회귀 핀 있음).
///   규칙 밖 파일·디렉토리·심볼릭링크는 **읽지도 지우지도 않는다**.
/// - 오래된 것부터 처리한다. 예산이 모자라면 오래된 쪽이 먼저 회수되고 새 쪽이 다음 부트로 밀린다.
/// - 삭제 실패·예산 밖 항목은 `remaining`/`remaining_bytes` 에 그대로 남는다(성장 은폐 금지).
/// - "나이 초과분 강제 삭제 시도"는 별도 분기가 아니라 **전건 시도의 부분집합**이다 — 매 부트
///   모든 항목에 삭제를 시도하므로 나이 초과분은 반드시 시도된다. 나이 축은 그 시도가 **실패**
///   했을 때의 등급(승격)을 정한다.
#[cfg(any(windows, test))]
pub(crate) fn bound_update_trash(
    trash: &std::path::Path,
    now_secs: u64,
    remove: &mut dyn FnMut(&std::path::Path) -> bool,
    stats: &mut TrashBoundStats,
) {
    // ★격리함은 **우리가 만든 실제 디렉토리**여야 한다. 심볼릭링크·정션이면 삭제가 링크 너머의
    //   남의 트리로 새어나간다(오너 앵커 ④ — 살아있는 파일 삭제 금지). symlink_metadata 는 링크를
    //   따라가지 않으므로 여기서 정확히 갈린다. 링크면 회수를 포기한다 —
    //   "못 지우고 남는 것"은 다음 부트가 재시도할 수 있지만, "잘못 지운 것"은 되돌릴 수 없다.
    if !std::fs::symlink_metadata(trash).is_ok_and(|m| m.file_type().is_dir()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(trash) else {
        return; // 격리함 없음 = 유계 대상 없음(정상 · 대다수 부트가 여기)
    };
    // (mtime, size, path). DirEntry::metadata 는 심볼릭링크를 따라가지 않는다 — 링크는 is_file
    // 이 false 가 되어 여기서 탈락한다(링크 너머의 살아있는 파일을 지우는 경로 차단).
    let mut items: Vec<(u64, u64, std::path::PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_update_leftover(name) {
            continue; // 우리가 만든 이름이 아니면 남의 파일이다 — 무접촉
        }
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(now_secs); // mtime 을 못 읽으면 '방금'으로 간주 = 나이 승격 대상 아님(보수적)
        items.push((mtime, md.len(), p));
    }
    // 오래된 것부터. 동률은 경로로 깨서 부트 간 순서를 결정론으로 고정한다.
    items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));

    for (i, (mtime, size, p)) in items.iter().enumerate() {
        if i >= TRASH_MAX_RECLAIM_PER_BOOT {
            // 예산 소진 — 시도는 못 해도 **계수에는 남긴다**.
            stats.deferred += 1;
            stats.remaining += 1;
            stats.remaining_bytes = stats.remaining_bytes.saturating_add(*size);
            continue;
        }
        if remove(p) {
            stats.reclaimed += 1;
            continue;
        }
        stats.remaining += 1;
        stats.remaining_bytes = stats.remaining_bytes.saturating_add(*size);
        // 나이 축이 재는 것은 **격리함 체류시간**이다 — 격리 순간 `set_quarantine_stamp` 이 심어둔
        // 시각. 둘째 조건은 "방금 읽은 mtime 이 정말 우리 스탬프인가"를 되묻는다: **같은 값**을 다시
        // 쓰는 멱등 no-op 이라 시계를 되돌리지 않으면서, 시각을 **쓸 수 없는 파일**(권한·FS 미지원
        // → 격리 때도 스탬프가 실패했다는 뜻)을 걸러낸다. 그런 항목의 mtime 은 업스트림 아카이브가
        // 준 남의 시각이라 나이 근거가 될 수 없으므로 **신선한 쪽으로 접는다** — 스탬프 실패가 조기
        // 경고를 유발하는 방향은 금지다(경고는 조치 가능할 때만 울린다). 늦게 우는 것은 안전하고,
        // 일찍 우는 것은 오탐 배너 클래스의 재발이다.
        if now_secs.saturating_sub(*mtime) > TRASH_MAX_AGE_SECS && set_quarantine_stamp(p, *mtime) {
            stats.aged_stuck += 1;
        }
    }
}

/// 부트 1회의 잔해 유지보수 전체 — 스윕(설치 트리 → 격리함) → 유계(격리함 회수·상한) → 로그.
///
/// ★`main()` 의 `#[cfg(windows)]` 블록에는 **이 함수 호출 한 줄만** 남긴다. 실동작 전량을
/// `cfg(any(windows, test))` 로 끌어올린 이유는 컴파일 커버리지다 — 종전에는 호출·정책·포맷이
/// 전부 `cfg(windows)` 안에 있어 macOS·Linux 의 `cargo test --bin cysd` 로는 **컴파일조차 되지
/// 않았고**, Windows CI 는 `--bin cysd` 를 돌리지 않는다(release.yml 은 ubuntu/macOS 레인).
/// 즉 이 경로는 어느 레인에서도 타입체크되지 않는 사각지대였다. 이제 타 OS 의 단위 테스트가
/// 같은 코드를 컴파일하고 파일시스템 동작까지 실제로 돌린다.
///
/// 반환값은 호출자가 그대로 stderr 로 흘리는 로그 줄이다(등급은 진단·테스트용).
#[cfg(any(windows, test))]
pub(crate) fn run_update_leftover_maintenance(
    dir: &std::path::Path,
    now_secs: u64,
) -> Vec<(LeftoverLog, String)> {
    let trash = dir.join(UPDATE_TRASH_DIR);
    let mut remove = |p: &std::path::Path| std::fs::remove_file(p).is_ok();
    let mut relocate = |src: &std::path::Path, dest: &std::path::Path| {
        dest.parent()
            .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
            && std::fs::rename(src, dest).is_ok()
    };
    // ★W1 가드 — 스윕보다 **먼저**: 정식 부재 가족의 잔해는 쓰레기가 아니라 복구 재료다.
    //   승격(정식 재건)에 성공한 가족만 스윕에 되돌아가고, 실패·재료 전무 가족은 hold 로 동결된다.
    let mut promote =
        |src: &std::path::Path, dest: &std::path::Path| std::fs::rename(src, dest).is_ok();
    let mut guard = GuardStats::default();
    let hold = guard_canonical_binaries(dir, "exe", &mut remove, &mut promote, &mut guard);
    let mut sweep = SweepStats::default();
    sweep_update_leftovers(
        dir,
        12,
        &trash,
        now_secs,
        &hold,
        &mut remove,
        &mut relocate,
        &mut sweep,
    );
    // 격리함 회수·유계는 스윕 **뒤**에 — 이번 부트에 새로 격리된 것까지 계산에 들어가야 한다.
    let mut bound = TrashBoundStats::default();
    bound_update_trash(&trash, now_secs, &mut remove, &mut bound);
    // 격리함이 비었으면 흔적을 남기지 않는다(빈 디렉토리 제거 — 비어있지 않으면 실패=no-op).
    let _ = std::fs::remove_dir(&trash);
    // 가드 줄이 먼저다 — 복구·동결이 스윕보다 먼저 일어난 사건이고, 정식 부재는 가장 큰 소식이다.
    let mut lines = guard_log_lines(&guard);
    lines.extend(leftover_log_lines(&sweep, &bound, &trash));
    lines
}

/// 로그 등급. `Loud` 만이 사용자에게 "봐야 할 것이 있다"고 말한다.
#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LeftoverLog {
    /// 관측 기록 — 사후 진단용. 조치 불요.
    Info,
    /// ⚠ — 사람이 봐야 하는 이상. **아껴 쓴다**(남발하면 경고의 신호가치가 죽는다).
    Loud,
}

/// 부트 1회의 잔해 처리 결과를 로그 줄로 환원한다(순수 · 회귀 핀).
///
/// ★이 함수가 **경고 등급의 유일한 결정처**다. 종전에는 등급 판정이 `#[cfg(windows)]` 블록
/// 안에만 있어서 macOS·Linux 의 `cargo test` 로는 컴파일조차 되지 않았다 — 정책이 테스트
/// 사각지대에 갇혀 있었다는 뜻이다. 순수 함수로 끌어내 등급 자체를 박제한다.
#[cfg(any(windows, test))]
pub(crate) fn leftover_log_lines(
    sweep: &SweepStats,
    bound: &TrashBoundStats,
    trash: &std::path::Path,
) -> Vec<(LeftoverLog, String)> {
    let mut out = Vec::new();
    // ① info — 이번 부트에 무슨 일이 있었는지 + 격리함의 현재 점유. 격리함에 남은 것은 평시엔
    //    **정상 대기**(봉쇄 성공 · 홀더가 죽기를 기다리는 설계된 상태 · 사용자 조치 불요)이므로
    //    여기까지만 말한다. 아무 일도 없었으면 한 줄도 내지 않는다.
    if sweep.removed + sweep.quarantined + sweep.stuck + bound.reclaimed + bound.remaining > 0 {
        let deferred = if bound.deferred > 0 {
            format!(", {} deferred", bound.deferred)
        } else {
            String::new()
        };
        out.push((
            LeftoverLog::Info,
            format!(
                "[cysd] update leftovers: removed={} quarantined={} stuck={} · trash: reclaimed={} pending={} ({} KiB{}) at {}",
                sweep.removed,
                sweep.quarantined,
                sweep.stuck,
                bound.reclaimed,
                bound.remaining,
                bound.remaining_bytes / 1024,
                deferred,
                trash.display()
            ),
        ));
    }
    // ①-b info — 격리는 됐는데 **체류시계를 심지 못한** 항목이 있다. 그 항목은 나이 축에서
    //    '신선'으로 접히므로 지금 사용자가 할 조치가 없다(= loud 아님). 다만 나이 승격이 그만큼
    //    늦어진다는 사실 자체는 남긴다 — 침묵하면 사후에 "왜 안 울렸나"를 되짚을 근거가 사라진다.
    if sweep.stamp_failed > 0 {
        out.push((
            LeftoverLog::Info,
            format!(
                "[cysd] update trash: 격리 시각 기록 실패 {}건 — 해당 항목은 나이 축에서 '신선'으로 접는다(조기 경고 방지)",
                sweep.stamp_failed
            ),
        ));
    }
    // ② loud — 삭제도 격리도 실패 = **살아있는 설치 트리에 잔해가 그대로 남았다**(봉쇄 실패).
    //    등급 강등 대상이 아니다. 종전 코드는 삭제 실패를 무음 스킵해 "청소가 됐는지"를 로그만으로
    //    판정할 수 없었다(T4-6 진단 난항의 원인) — 침묵 금지.
    if sweep.stuck > 0 {
        out.push((
            LeftoverLog::Loud,
            format!(
                "[cysd] ⚠ 업데이트 잔해 {}개 회수 불가(삭제·격리 모두 실패) — 다음 기동이 재시도한다",
                sweep.stuck
            ),
        ));
    }
    // ③ loud — 격리함이 유계를 벗어났다. **여기까지 와야** 사람이 볼 값어치가 있다
    //    (상한·근거는 TRASH_MAX_* 상수 주석 · 판정처는 TrashBoundStats::over_bound).
    if bound.over_bound() {
        out.push((
            LeftoverLog::Loud,
            format!(
                "[cysd] ⚠ 격리함 유계 이탈: {}개/{} MiB 미회수(상한 {}개/{} MiB), 나이초과 미회수 {}개 — \
                 오래 열어둔 세션(터미널·셸)을 닫고 재기동하면 회수된다. 계속되면 {} 를 확인하라",
                bound.remaining,
                bound.remaining_bytes / (1024 * 1024),
                TRASH_MAX_ENTRIES,
                TRASH_MAX_BYTES / (1024 * 1024),
                bound.aged_stuck,
                trash.display()
            ),
        ));
    }
    out
}

/// Claude Code 세션 안에서 spawn된 데몬이 그 세션의 정체성 env를 PTY 자식들에게
/// 물려주면, pane의 claude가 **child-session 모드**(부모 세션 종속)로 동작해 트랜스크립트
/// .jsonl을 영속하지 않는다 — 복원(restore)·recall·사용량 관측(T5)이 전부 깨진다
/// (2026-06-13 실측: 데몬을 `cys ping`으로 claude Bash에서 재기동하자 신규 노드 4종
/// 전부 트랜스크립트 미생성, env에 CLAUDE_CODE_SESSION_ID=부모세션 확인).
/// 데몬은 어떤 환경에서 spawn되든 자식에게 세션 정체성을 누설하면 안 된다 — 기동 즉시 제거.
fn scrub_claude_session_env() {
    const LEAKY: [&str; 5] = [
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SSE_PORT",
    ];
    for k in LEAKY {
        if std::env::var_os(k).is_some() {
            std::env::remove_var(k);
            eprintln!("[cysd] scrubbed leaky claude session env: {k}");
        }
    }
}

/// ★SEAL-1 층3 배선 지점(sync main). `#[tokio::main]` 은 `async_main` 으로 내려가 있고, 여기서
/// **런타임(=워커 스레드)이 만들어지기 전에** 프로세스 env 를 봉인한다 — `set_var` 는 프로세스
/// 전역이라 스레드가 살아 있는 동안 쓰면 경합한다(lib.rs `seal_python_bytecode_in_process` 계약).
/// 이 한 줄로 데몬의 **모든** 자손이 덮인다: pane(층2 가 이미 명시 주입) + 층1·층2 가 닿지 않는
/// 임의 명령 경로 — `channels::spawn_bridge`(사용자 bridge_cmd)·`accounts` cmd 어댑터(주기 폴링).
fn main() {
    cys::seal_python_bytecode_in_process();
    async_main();
}

#[tokio::main]
async fn async_main() {
    // ★수리 세대 부팅 로그 — 릴리스 게이트 마커의 확정 임베드 지점(v4).
    // main() 첫 실행 경로라 어떤 타깃·최적화에서도 데드코드 제거가 불가능하다
    // (v3 status() 참조는 x86_64 코드젠에서 함수째 소거됨 — run 30367192331 실증).
    // 부수 효용: 데몬 stderr 로그 첫 줄에서 설치본 수리 세대를 즉시 판별.
    eprintln!(
        "[cysd] v{} {}",
        env!("CARGO_PKG_VERSION"),
        crate::schedule::FIX_GENERATION
    );
    scrub_claude_session_env();

    // ★W1(조기 단일 인스턴스 게이트): 소켓 경로 확정 직후·pack 설치보다 먼저 단일 인스턴스 게이트를
    // 통과시킨다. 목적 — 락/싱글턴 경쟁의 **패자**가 상태를 오염시키는 부트 부수효과 전에 죽게 하는 것.
    // (게이트 뒤 부수효과: Daemon::new 의 operator.token 디스크 덮어쓰기·feed.jsonl compaction, 워치독·
    //  스케줄러·오피스 브리지 spawn, pack install, daemon.started 발행 등.) ★리뷰어1 F2: 패자의 잔여
    // 부수효과는 상태디렉터리 mkdir/chmod 0o700(멱등·무해)뿐 — operator.token·feed.jsonl 등 상태 파일과
    // 프로세스 spawn 은 무접촉이다. 과거엔 락 획득을 accept_loop 진입까지 미뤄 패자가 부수효과 전량을
    // 실행한 뒤에야 죽었다 → launchd KeepAlive 재기동 폭주 시 패자가 매번 operator.token 을 덮어써 라이브
    // 데몬 메모리 토큰과 불일치 → GUI 승인 Feed 우회가 무력화되어 Allow 전멸.
    let socket_path = cys::socket_path();

    // unix: flock 기반 startup 락 — 경합 시 hung 홀더는 데드맨이 회수·인수, 건강한 홀더/구 락파일은
    // fail-closed exit. 락 파일 핸들은 이 main 스코프에서 데몬 수명 동안 보유한다(핸들 drop = flock 해제 =
    // 게이트 소멸이므로 절대 조기 drop 금지 — accept_loop 는 반환하지 않아 main 종료까지 살아있다).
    #[cfg(unix)]
    let _lock_file = {
        use std::os::unix::fs::PermissionsExt;
        // 상태 디렉터리 선생성: 락 파일이 이 디렉터리에 놓이므로 락 획득 전에 반드시 존재해야 한다
        // (소유자 전용 0o700 — transcripts.db·feed.jsonl·소켓을 같은 UID로 봉인).
        if let Some(dir) = socket_path.parent() {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        // ★W3: 경합 시 단순 exit(1)이 아니라, 홀더가 hung(무응답 + heartbeat stale)이면 데드맨이
        // 회수·인수한다. 건강한 홀더/구 락파일(pid 미상)은 fail-closed로 exit(무손실·오살상 차단).
        let lock_path = socket_path.with_extension("lock");
        let state_dir = socket_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let lock = acquire_startup_lock(&lock_path, &socket_path, &state_dir);
        // ★W3 heartbeat: 승자만 주기적으로 mtime을 갱신한다 → 런타임이 wedge되면 interval 태스크가
        // 진행하지 못해 자연히 stale이 되고, 다음 경합자의 데드맨이 dead로 판정할 수 있다.
        // 기동 창(락 획득~첫 주기 touch)은 claim_lock의 동기 초기 touch가 방어한다.
        {
            let hb = deadman::heartbeat_path(&state_dir);
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(deadman::HEARTBEAT_INTERVAL);
                loop {
                    tick.tick().await;
                    deadman::touch_heartbeat(&hb);
                }
            });
        }
        lock
    };

    // windows: named pipe first-instance 선점 = 데몬 싱글턴 가드. 조기에 first 인스턴스를 만들어
    // accept_loop 로 넘겨 재사용한다(probe-후-close-재open 레이스 없이 그대로 리스너 풀에 편입).
    // 선점 실패(이미 홀더 존재)는 기존 즉사 의미 유지 — eprintln 후 exit 1.
    #[cfg(windows)]
    let first_pipe = {
        let pipe_name = socket_path.to_string_lossy().into_owned();
        match create_pipe_instance(&pipe_name, true) {
            Ok(s) => s,
            Err(e) => {
                // ★리뷰어1 F3: 구 panic!(exit 101) → exit(1) 통일 — 즉사 의미는 동일하되 종료코드를
                // unix 패자(acquire_startup_lock 의 exit(1))와 일치화한다.
                eprintln!("error: another cysd already owns the pipe {pipe_name}: {e}");
                std::process::exit(1);
            }
        }
    };

    // windows .prev sweep 은 위 싱글턴 게이트 **뒤**에서 수행 — 승자만 잔해를 정리한다(패자는 이미 즉사).
    // ★무중단 rename-swap 잔해 청소(nsis-hooks.nsh의 짝): 업데이트가 잠긴 파일을 죽이는 대신
    // <이름>.prev*(cysd/cys 고정 체인 + unlock-sweep의 <이름>.prev<rand> — msys-2.0.dll 등 세션이
    // 로드한 runtime 이미지)로 밀어두므로, 새 cysd 기동 시 설치 트리를 재귀 순회하며 잔해를
    // 회수한다. 삭제가 실패하는 경우(=아직 살아있는 홀더가 이미지로 매핑 중)의 처리는
    // sweep_update_leftovers 주석 참조 — **격리(rename)로 완결**한다. 깊이 상한 12.
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for (_, line) in run_update_leftover_maintenance(dir, now) {
                eprintln!("{line}");
            }
        }
    }
    // ★G34(W3) 데몬측 (소켓,팩) 정합 검사 — 이 데몬이 어떤 레인을 어떤 팩으로 서빙하는지 **기동
    // 로그에 못박는다**. 부서 소켓+본부 팩 조합은 부서 부트를 exit 8 로 영구 차단하고 본부 팩을
    // 교차 서빙한다(F1 격리 붕괴·schedule 중복 발화). CLI 측은 스폰 전에 거부하지만(ensure_daemon_
    // lane_pack), GUI·launchd·수동 기동 등 다른 진입점이 남으므로 데몬 자신도 판정해 기록한다.
    // ※비치명(로그): 이미 뜬 데몬을 죽이는 것은 이 웨이브의 범위가 아니다 — 진단 가시성이 목적이다.
    {
        let is_dept = cys::is_dept_socket(&socket_path);
        let pack = cys::pack::pack_dir();
        let pack_is_dept = pack
            .file_name()
            .map(|n| n.to_string_lossy().starts_with("pack-dept-"))
            .unwrap_or(false);
        if is_dept != pack_is_dept {
            eprintln!(
                "[cysd] ⚠ 레인↔팩 불일치: socket={} (dept={is_dept}) pack={} (dept={pack_is_dept}) \
                 — 부서 데몬은 `cys-dept launch <name>`(CYS_SOCKET+CYS_PACK_DIR 쌍)으로 기동해야 한다. \
                 이 조합은 부서 부트 차단·팩 교차 서빙을 유발한다.",
                socket_path.display(),
                pack.display()
            );
        } else {
            eprintln!(
                "[cysd] lane: socket={} pack={} (dept={is_dept})",
                socket_path.display(),
                pack.display()
            );
        }
    }
    // crash recovery(§7-⑤): 직전 pack-update가 apply 도중 죽어 남긴 orphan 저널을 install(false)
    // **이전에** 자가치유한다(미커밋=rollback / 커밋완료=정리). 순서가 중요 — install(false)가
    // 부분반영 트리 위에서 돌면 안 되므로 반드시 선행한다.
    // ★리뷰어1 F1: 조기 락 전진(W1)으로 "락 보유~소켓 bind" 창이 이 pack recover/install 동기 블로킹을
    // 통째로 포함하게 됐다 — 단일 워커(1코어)에서 이 블로킹이 tokio 워커를 45초(HEARTBEAT_STALE_THRESHOLD)+
    // 굶기면 heartbeat interval 태스크가 못 돌아 stale → 경합자 데드맨이 정당한 승자를 Dead 오판·SIGKILL 할
    // 수 있다. spawn_blocking 으로 별도 블로킹 풀에 태워 main 태스크가 yield 해도 interval 이 계속 돌게 한다.
    match tokio::task::spawn_blocking(cys::pack::recover_pack_journal).await {
        Ok(Ok(true)) => eprintln!("[cysd] pack-update orphan journal recovered (self-heal)"),
        Ok(Ok(false)) => {}
        Ok(Err(e)) => eprintln!("[cysd] pack journal recovery skipped: {e}"),
        Err(e) => eprintln!("[cysd] pack journal recovery task failed: {e}"),
    }
    // 온보딩②: 팩이 이 바이너리 버전으로 미커밋일 때만 자동 설치 — 신규 머신·바이너리 업그레이드·
    // 팩 소실(.pack-version/매니페스트 부재 = 게이트 개방)이 실행 조건. launch-agent·디렉티브·acl이
    // "init-pack을 아는 사람"에게만 동작하는 것을 없앤다는 원목적은 유지된다(보존 모드·사용자 파일 불가침).
    // ★게이트(pack_current_for): 평시 부트는 stat 2회로 조기 반환 — 부서 데몬 N개·RestartOnFailure
    // 재기동·로그온 자동기동마다 전량 스윕(320파일 read+해시)이 돌던 비용 제거(2026-07-12 Win11 이슈 실측).
    // 손상+마커 무결 상태의 치유는 cys init-pack/pack-update/doctor --fix 명시 경로가 담당한다
    // (매 부트 전량 치유는 seed-once 원복 사고(7-12)의 원인 기전 — 의도적 축소).
    if !cys::pack::pack_current_for(env!("CARGO_PKG_VERSION")) {
        // ★리뷰어1 F1: install(false)도 동기 블로킹(최대 320파일 read+해시+write)이라 위 heartbeat 굶김
        // 위험이 동일 — spawn_blocking 으로 분리한다. (pack_current_for 게이트는 stat 2회라 동기 유지.)
        // W0-d: cysd 부팅 자동설치는 라이브 팩 쓰기 프로덕션 진입점 — 인가 부여.
        match tokio::task::spawn_blocking(|| {
            cys::pack::install(false, Some(cys::pack::PackWriteAuth::production()))
        })
        .await
        {
            Ok(Ok((written, _))) if written > 0 => eprintln!(
                "[cysd] CYSJavis Pack: {written} file(s) installed at {}",
                cys::pack::pack_dir().display()
            ),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("[cysd] pack auto-install skipped: {e}"),
            Err(e) => eprintln!("[cysd] pack auto-install task failed: {e}"),
        }
    }
    let daemon = Daemon::new(socket_path.clone());
    // ★R1 배달 원장: 이 데몬 인스턴스 표식을 팩 계약 상태 디렉터리에 쓴다(best-effort).
    //   임무 대장(javis_mission)이 이 값을 **세션 결박**에 쓴다 — 데몬이 재기동하면 과거 세션의
    //   오너 임무는 무효가 된다(적발 (a): ts 를 기록만 하고 읽지 않아 과거 임무가 무기한 유효했다).
    //   실패해도 기동을 막지 않는다: 표식 부재는 판독자 쪽에서 'TTL 만 적용'으로 degrade 된다.
    delivery::write_epoch(&socket_path);
    // ★R4 fail-open ② 봉합: 기동 표식 1줄을 원장에 append 해 "정상 원장은 절대 0바이트가 아니다"
    //   를 성립시킨다. 이것이 있어야 판독자가 '존재하지만 0바이트 = 손상'을 fail-closed 로
    //   판정할 수 있다(종전엔 빈 파일이 LEDGER_OK 로 통과해, 원장을 비우기만 하면 게이트가 열렸다).
    //   실패는 기동을 막지 않되 **조용히 넘기지 않는다** — 흔적을 stderr 에 남긴다.
    if let delivery::Outcome::Failed(why) = delivery::write_boot_sentinel(&socket_path) {
        eprintln!(
            "cysd: ★배달 원장 기동 표식 기록 실패({why}) — 판독자가 이 레인 원장을 '손상'으로 \
             볼 수 있다(임무 게이트 fail-closed). 상태 디렉터리 권한을 확인하라."
        );
    }

    governance::spawn_watchdog(Arc::clone(&daemon));
    // ★(U-23) 부트 감독자 — **watchdog 과 별도 태스크**다. 근본원인 R3(감독자 없는 단발 체인)의
    //   해소 지점이며, 훅이 잘려도 부트가 살아남게 하는 유일한 층이다.
    //   ★여기서 `spawn_watchdog` 다음 줄에 두는 것은 순서 의존이 아니라 가독성이다 — 두 태스크는
    //     서로의 상태를 읽지 않고 cadence 도 다르다(5초 vs 3초). watchdog 틱 본문(동기 클로저)에
    //     이 일을 얹으면 부트 1회가 큐 배달·승인 격상·데드맨을 수십 초 정지시킨다(치명위험 ②③).
    //   ★롤백: `CYS_BOOT_GATES=0`(마스터) 또는 `CYS_BOOT_SUPERVISOR=0` → 이 태스크가 뜨지 않는다.
    boot_supervisor::spawn(Arc::clone(&daemon));
    // ★B2-1(W3): built-in phoenix 잡을 부트 시 idempotent ensure — schedule.json 이 user-owned 로 전환돼
    //   팩 배달로는 built-in 잡을 갱신할 수 없으므로 코드가 upsert 한다(부재 생성·구버전 갱신·중복 0). 스케줄러 기동 전.
    schedule::ensure_builtin_jobs();
    schedule::spawn_scheduler(Arc::clone(&daemon));
    usage::spawn_usage_collector(Arc::clone(&daemon));
    usage::spawn_agy_collector(Arc::clone(&daemon));
    // CC v2 WS-A: 계정 발견(프로필 스캔)+스냅샷 예열 — 관측 전에도 전 계정이 CC에 보인다.
    // 전부 fail-open(파일 부재·파싱 실패=빈 뷰) — 부트체인 비치명.
    accounts::seed_known(&daemon);
    accounts::spawn_custom_adapters(Arc::clone(&daemon));
    // CC v2 WS-B: 스킬 run 생애주기 — 이전 데몬의 열린 run 정리 후 전이 워처 기동.
    skillrun::reconcile_boot(&daemon);
    skillrun::spawn_watcher(Arc::clone(&daemon));
    // CC "🏢 오피스" 탭의 상시 가용성 — 메타버스 오피스 브리지(127.0.0.1:8642) 자동기동.
    spawn_office_bridge(crate::state::state_dir(&socket_path));
    // C0: 채널 부팅 재조정(고아 선-kill→새 토큰 재스폰) — 이벤트버스·state 준비 후(§2.1-2).
    // 불사조 복원 프로토콜의 "채널 재조정" 단계. 그 다음 주기 sweep(재배달·타임아웃·재스폰) 등록.
    channels::reconcile(&daemon);
    channels::spawn_channel_sweep(Arc::clone(&daemon));
    // 셧다운 경로: 원장은 메모리 전용이라 데몬이 죽으면 scoped 프로세스를 아무도 회수하지
    // 못한다 — SIGTERM/SIGINT 때 scoped 그룹을 전부 정리한 뒤 종료한다.
    #[cfg(unix)]
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let (Ok(mut term), Ok(mut int)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
            ) else {
                return;
            };
            tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            shutdown_cleanup(&d, "signal");
            std::process::exit(0);
        });
    }
    // Windows: SIGTERM/SIGINT가 없으므로 콘솔 제어 이벤트로 같은 회수를 건다.
    // Ctrl-C·콘솔 닫힘·로그오프/셧다운(=catchable) 시 scoped 그룹을 정리한다.
    // (taskkill /F는 TerminateProcess라 어떤 핸들러도 못 받음 — 그 경로는 호출측
    //  taskkill /T·원장 정리의 몫. 여기선 unix가 잡던 모든 catchable 종료를 대칭화.)
    #[cfg(windows)]
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            use tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_shutdown};
            let (Ok(mut cc), Ok(mut close), Ok(mut shutdown)) =
                (ctrl_c(), ctrl_close(), ctrl_shutdown())
            else {
                return;
            };
            tokio::select! {
                _ = cc.recv() => {},
                _ = close.recv() => {},
                _ = shutdown.recv() => {},
            }
            shutdown_cleanup(&d, "console_event");
            std::process::exit(0);
        });
    }
    daemon.bus.publish(
        "daemon.started",
        "system",
        None,
        json!({"pid": std::process::id(), "socket": socket_path.to_string_lossy()}),
    );

    eprintln!(
        "cysd (CYSJavis terminal daemon) listening on {}",
        socket_path.display()
    );
    #[cfg(unix)]
    accept_loop(daemon, &socket_path).await;
    // windows: main()에서 조기 선점한 first 파이프 인스턴스를 넘겨 리스너 풀에 재사용시킨다.
    #[cfg(windows)]
    accept_loop(daemon, &socket_path, first_pipe).await;
}

/// 종료 직전 회수: 원장의 scoped 그룹을 전부 죽이고, stopping 이벤트 발행 후
/// 소켓 파일을 제거한다. unix·windows 양쪽 종료 핸들러가 공유한다 (크로스플랫폼 대칭).
/// (windows named pipe엔 제거할 파일이 없어 remove_file은 무해한 no-op이 된다.)
fn shutdown_cleanup(daemon: &Arc<Daemon>, reason: &str) {
    let scoped = governance::collect_scoped_for_shutdown(&daemon.ledger.lock().unwrap());
    for (pid, pgid) in scoped {
        governance::kill_group_or_pid(pid, pgid);
    }
    daemon
        .bus
        .publish("daemon.stopping", "system", None, json!({"reason": reason}));
    let _ = std::fs::remove_file(&daemon.socket_path);
}

#[cfg(unix)]
async fn accept_loop(daemon: Arc<Daemon>, socket_path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    // ★W1: startup 락 획득·heartbeat spawn·상태 디렉터리 선생성은 부트 부수효과보다 먼저 실행돼야
    // 하므로 main()으로 전진했다(경쟁 패자가 부수효과 실행 전 즉사). 락 파일 핸들은 main 스코프에서
    // 데몬 수명 동안 보유된다. 여기(accept_loop)에는 소켓 바인드·수신 준비만 남긴다.
    // Refuse to start if a live daemon already owns the socket (중복 기동 방지 — 거버넌스 철학).
    if socket_path.exists() {
        if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
            eprintln!(
                "error: another cysd is already listening on {}",
                socket_path.display()
            );
            std::process::exit(1);
        }
        let _ = std::fs::remove_file(socket_path);
    }
    let listener = tokio::net::UnixListener::bind(socket_path)
        .unwrap_or_else(|e| panic!("bind {} failed: {e}", socket_path.display()));
    // 소켓 파일은 소유자만 read/write — 인증 없는 제어 채널을 같은 UID로 한정한다.
    // (master·worker·gemini·codex 노드는 모두 오너 UID로 도는 단일 사용자 구조)
    let _ = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600));

    // ★W2 콜드부트 자동 복원: 소켓 바인드·수신 준비가 끝난 '이후'에만 1회 발화한다(자식
    // phoenix가 이 데몬 소켓으로 즉시 RPC할 수 있어야 하므로 바인드 성공이 선행 조건).
    // raw `cys restore`가 아니라 phoenix를 태워 desired_roster·묘비·회로차단기·저널을 경유한다.
    // ★P0-7(D1/W5): prune + auto-restore 를 공통 post_listen_boot 로 — Windows accept_loop 와 동일 함수 호출
    //   (한쪽만 배선되던 미배선 결함 봉인). state_dir 은 함수 내부에서 canonical 매핑으로 재계산.
    post_listen_boot(socket_path, &daemon);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // T1-3 발신자 신원: 커널이 보증하는 peer pid (자기신고 from의 검증 토대)
                let caller_pid = peer_pid(&stream);
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    handle_connection(daemon, Box::new(stream) as Stream, caller_pid).await;
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

/// ★W3 startup lock 획득 — 경합 시 데드맨 에스컬레이션(hung 홀더 회수·인수)까지 수행한다.
/// 성공 시 락파일에 자기 pid 기록 + heartbeat 초기 touch 후 락 핸들 반환(데몬 수명 동안 보유).
/// 락 파일 자체를 못 열면 None(기존 동작 — connect 점검만으로 진행).
/// 회수 실패·건강한 홀더·구 락파일(pid 미상)은 fail-closed로 exit(1)(dedupe 로그).
#[cfg(unix)]
fn acquire_startup_lock(
    lock_path: &std::path::Path,
    socket_path: &std::path::Path,
    state_dir: &std::path::Path,
) -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
    {
        Ok(f) => f,
        Err(_) => return None, // 락 파일 생성 실패 — 기존 connect 점검만으로 진행
    };
    let try_flock =
        |f: &std::fs::File| unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 };

    if try_flock(&file) {
        deadman::claim_lock(&mut file, state_dir);
        return Some(file);
    }

    // ★WS-7: try_flock 실패는 곧바로 데드맨 판정으로 넘기지 않고 **지수 백오프+지터로 재시도**한다.
    // 근거: `cys doctor`가 진단 스팬 동안 같은 락을 순간 보유한다(cys.rs diag_orphan_socket·
    // diag_stale_lock). 재시도가 없으면 그 순간 부팅한 데몬이 홀더를 dead로 오판→회수 실패→
    // `dead-holder-reclaim-failed` **오사유로 exit(1)** 하고, 30회당 1줄 로그 억제(deadman.rs:197)와
    // launchd 10s 재기동이 겹쳐 **무로그 crashloop**이 된다.
    // ★적용 범위 엄수: 재시도는 **try_flock에만** 붙인다. judge_holder의 입력(holder_pid·responded·
    // hb_stale)과 진리표는 무접촉이다 — 데드맨 계약(X5·X6)을 침범하지 않는다.
    for backoff in lock_retry_schedule() {
        std::thread::sleep(jittered(backoff));
        if try_flock(&file) {
            deadman::claim_lock(&mut file, state_dir);
            return Some(file);
        }
    }

    // 경합: 현재 홀더 상태 진단(pid·소켓 응답·heartbeat 신선도) → 판정.
    let holder_pid = deadman::read_holder_pid(lock_path);
    let responded = deadman::probe_holder(socket_path, deadman::PROBE_TIMEOUT);
    let hb_stale = deadman::heartbeat_stale(
        &deadman::heartbeat_path(state_dir),
        deadman::HEARTBEAT_STALE_THRESHOLD,
    );
    match deadman::judge_holder(holder_pid, responded, hb_stale) {
        deadman::HolderVerdict::Dead => {
            // 홀더 hung 확정 → 회수(SIGTERM→SIGKILL, cysd 검증 후) → 락 1회 재획득 시도.
            let pid = holder_pid.expect("Dead 판정은 pid 존재를 함의");
            if deadman::reclaim_from_dead_holder(pid, deadman::RECLAIM_GRACE, deadman::pid_is_cysd)
                && try_flock(&file)
            {
                deadman::claim_lock(&mut file, state_dir);
                eprintln!("[cysd] deadman: reclaimed startup lock from dead holder (pid {pid})");
                return Some(file);
            }
            log_lock_loss(state_dir, lock_path, "dead-holder-reclaim-failed");
            std::process::exit(1);
        }
        deadman::HolderVerdict::Healthy => {
            log_lock_loss(state_dir, lock_path, "healthy-holder");
            std::process::exit(1);
        }
        deadman::HolderVerdict::FailClosed => {
            // 구 락파일(pid 미상) — 오살상 방지 위해 개입하지 않고 exit.
            log_lock_loss(state_dir, lock_path, "unknown-holder-pid");
            std::process::exit(1);
        }
    }
}

/// startup flock 재시도 백오프 스케줄(순수 — 테스트 가능). 기본 50→100→200→400→800ms(총 1550ms).
/// 총 예산이 1초를 넘어야 doctor의 순간 보유(진단 2건 연속 + 테스트 노브)를 흡수한다.
/// `CYS_LOCK_RETRY_MS`로 총 예산을 주입할 수 있다(테스트 결정론용 — 0이면 재시도 없음).
fn lock_retry_schedule() -> Vec<std::time::Duration> {
    schedule_for_budget(
        std::env::var("CYS_LOCK_RETRY_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1550),
    )
}

fn schedule_for_budget(budget_ms: u64) -> Vec<std::time::Duration> {
    let mut out = Vec::new();
    let (mut used, mut step) = (0u64, 50u64);
    while used < budget_ms {
        let d = step.min(budget_ms - used);
        out.push(std::time::Duration::from_millis(d));
        used += d;
        step = (step * 2).min(800);
    }
    out
}

/// 백오프에 ±20% 지터 — 여러 데몬이 동시에 재시도해 같은 순간에 몰리는 thundering herd를 흩는다.
/// 신규 크레이트 금지 계약을 지키려 시스템 시각 나노초를 엔트로피로 쓴다(공정성 요구 없음).
fn jittered(d: std::time::Duration) -> std::time::Duration {
    let ms = d.as_millis() as u64;
    if ms < 5 {
        return d;
    }
    let span = ms / 5;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|x| x.subsec_nanos() as u64)
        .unwrap_or(0);
    std::time::Duration::from_millis(ms + (nanos % (2 * span + 1)) - span)
}

/// ★W3 crashloop 로그 dedupe — 동일 사유 연속 패배는 N회당 1줄만(누적 카운트 병기).
/// 상태는 state_dir 파일 기반(프로세스가 매번 새로 뜨므로 in-memory 불가).
#[cfg(unix)]
fn log_lock_loss(state_dir: &std::path::Path, lock_path: &std::path::Path, reason: &str) {
    let state_path = state_dir.join("lockloss.state");
    let prev = std::fs::read_to_string(&state_path).ok();
    let (should_log, count, new_state) =
        deadman::dedupe_loss_log(prev.as_deref(), reason, deadman::LOCK_LOSS_LOG_EVERY_N);
    let _ = std::fs::write(&state_path, new_state);
    if should_log {
        eprintln!(
            "error: another cysd holds the startup lock ({}) — reason={reason}, occurrence #{count}",
            lock_path.display()
        );
    }
}

/// ★W2 콜드부트 자동 복원 판정(순수 함수 — 부수효과 없음, 단위 테스트 가능).
/// opt-out(CYS_NO_AUTORESTORE)이 아니면 항상 Ready — ★B1: phoenix 는 바이너리 임베드본이 권위이므로
/// 디스크 팩 phoenix 부재가 "미설치 skip"이 아니다(임베드 추출로 실행). args[0]=디스크 phoenix(폴백 후보).
#[derive(Debug, PartialEq)]
enum AutoRestore {
    /// CYS_NO_AUTORESTORE=1 — 사용자가 콜드부트 복원을 껐다.
    OptedOut,
    /// 스폰 대상: `python3 <phoenix> restore --auto`. args[0]=디스크 phoenix(B1 폴백 후보).
    /// ★W1/B3(§5-1): env 에 PHOENIX_CYS(exe 옆 cys 절대경로)·PATH(runtime 선두주입)를 주입한다 —
    /// GUI/데몬 최소 PATH(/usr/bin:/bin:…)에서 phoenix 가 `cys` 를 못 찾아 FileNotFoundError→exit 1
    /// 침묵사하던 라이브 결함(2026-07-06 실증)의 근원 수리. 순수 판정이라 단위 테스트로 env 를 검증한다.
    Ready {
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
}

/// ★W1/B3: exe_dir·current_path 를 인자로 받는 순수 함수(부수효과 없음·env 주입까지 단위 테스트 가능).
/// ★W6/E1: socket_path 를 phoenix 에 `--socket` 으로 명시 전달 — phoenix 의 상태 디렉터리(topology/desired/저널)가
/// 데몬 자신의 소켓에서 파생되게 한다. 프로덕션 무변경(dirname(라이브 소켓)==phoenix LIVE_STATE 로 동일 해석)이면서,
/// 격리 상태 디렉터리 E2E(데몬 교체 시뮬레이션)에서 phoenix 가 올바른 격리 소켓/상태를 타게 하는 enabler다.
fn decide_auto_restore(
    pack_dir: &std::path::Path,
    opted_out: bool,
    exe_dir: &std::path::Path,
    current_path: &str,
    socket: &str,
) -> AutoRestore {
    if opted_out {
        return AutoRestore::OptedOut;
    }
    // ★B1: 디스크 존재 게이트 제거 — 임베드본이 권위(디스크 부재여도 추출 실행). 이 경로는 폴백 후보다.
    let phoenix = pack_dir.join("bin").join("javis_phoenix.py");
    let mut env: Vec<(String, String)> = Vec::new();
    // PHOENIX_CYS: 데몬 exe 옆 동봉 cys 절대경로. 실존할 때만 주입한다(없으면 phoenix 의 which→표준경로
    // 폴백에 맡긴다 — 존재하지 않는 경로를 강제 주입해 재차 FileNotFoundError 를 만들지 않는다).
    let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
    let cys_path = exe_dir.join(cys_name);
    if cys_path.is_file() {
        env.push((
            "PHOENIX_CYS".to_string(),
            cys_path.to_string_lossy().into_owned(),
        ));
    }
    // PATH 재합성 — pane 자식(state.rs)과 동일 유틸 재사용(중복 구현 금지). 무변경이면 None(무주입).
    if let Some(newp) = cys::runtime_prefixed_path(exe_dir, current_path) {
        env.push(("PATH".to_string(), newp));
    }
    // ★B3(§2 축B): 인터프리터 절대경로 해석 — 동봉 runtime python3 우선(win runtime\python\python3.exe /
    // mac Resources/runtime/python/bin/python3), 없으면 "python3" 리터럴(PATH 폴백). 순정 Windows(python3 부재)·
    // mac CLT 미설치 소비자에서 첫 스폰 단절(P0-7·P1-9)을 절대경로로 끊는다. PATH 선두주입과 이중 방어.
    let python = bundled_python3(exe_dir).unwrap_or_else(|| "python3".to_string());
    // args[0]=디스크 phoenix(폴백 후보) · 이후 `--socket <s> restore --auto`. spawn 이 args[0]을 실 실행원으로 교체.
    AutoRestore::Ready {
        program: python,
        args: vec![
            phoenix.to_string_lossy().into_owned(),
            "--socket".to_string(),
            socket.to_string(),
            "restore".to_string(),
            "--auto".to_string(),
        ],
        env,
    }
}

/// 메타버스 오피스 브리지(팩 javis_hud_bridge.py · 127.0.0.1 한정) 자동기동 — CC "🏢 오피스" 탭이
/// 수동 python3 기동 없이 항상 열리게 한다. 단일 인스턴스 가드: HUD 포트가 이미 listen 중이면
/// (선행 cysd·수동 기동) 스폰하지 않는다 — 동일 서버 누적이 구조적으로 0(자원 거버넌스 '누적·미종료' 차단).
/// 사망·부재는 60s 주기 재확인이 이어받고(KeepAlive), cysd 정상 종료 시 kill_on_drop이 자식을 동반 정리한다.
/// CYS_NO_OFFICE_BRIDGE=1 opt-out · 팩에 브리지 부재(구팩)면 조용히 skip.
/// python 해석·PATH·cys 주입은 auto-restore(★B3)와 동일 SOT(bundled_python3·runtime_prefixed_path).
fn spawn_office_bridge(state_dir: std::path::PathBuf) {
    if cys::env_compat("CYS_NO_OFFICE_BRIDGE").map(|v| v == "1").unwrap_or(false) {
        eprintln!("[cysd] office-bridge skipped (CYS_NO_OFFICE_BRIDGE=1)");
        return;
    }
    let script = cys::pack::pack_dir().join("bin").join("javis_hud_bridge.py");
    if !script.is_file() {
        return; // 구팩(브리지 미배포) — 다음 팩 업데이트가 채운다.
    }
    let port: u16 = std::env::var("HUD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8642);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    tokio::spawn(async move {
        let exe_dir_ref = exe_dir.as_deref().unwrap_or_else(|| std::path::Path::new("."));
        let python = bundled_python3(exe_dir_ref).unwrap_or_else(|| "python3".to_string());
        let log_path = state_dir.join("office-bridge.log");
        loop {
            // 단일 인스턴스 가드 — 이미 서비스 중(선행 데몬·수동 기동)이면 스폰하지 않고 재확인만.
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
            let mut cmd = tokio::process::Command::new(&python);
            cmd.arg(&script)
                // 브리지의 cys 호출이 라이벌 데몬을 autostart하는 재귀 차단(auto-restore와 동일 계약).
                .env("CYS_NO_AUTOSTART", "1")
                // ★SEAL-1: 브리지는 **장수 프로세스**라 import 표면이 가장 넓다 — 번들 python 이
                // 여기서 `.pyc` 를 쓰면 코드서명 봉인이 깨진다. tokio 빌더라 python_command 팩토리를
                // 못 쓰므로 같은 상수를 직접 소비한다(규약 산재 아님 · lib.rs ENV_PY_NO_BYTECODE).
                .env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON)
                // 런타임 상태는 팩 트리 밖으로(팩 본체 오염 0 — 팩 편입 계약 HUD_STATE_DIR).
                .env("HUD_STATE_DIR", state_dir.join("office-bridge"))
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            {
                // Windows: 콘솔 없는 cysd가 콘솔 자식(python3.exe)을 그냥 스폰하면 새 콘솔 창이
                // 할당된다(Win11 기본터미널=WT → AppData 경로 제목의 검은 상주 탭). 브리지는
                // 장수 프로세스라 앱 기동마다 빈 터미널 창이 함께 뜨던 실사고(2026-07-10)의 주범.
                use crate::state::HideConsole;
                cmd.hide_console();
            }
            if let Some(newp) =
                cys::runtime_prefixed_path(exe_dir_ref, &std::env::var("PATH").unwrap_or_default())
            {
                cmd.env("PATH", newp);
            }
            let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
            let cys_path = exe_dir_ref.join(cys_name);
            if cys_path.is_file() {
                cmd.env("HUD_CYS_BIN", &cys_path); // 사이드카 cys 절대경로(PHOENIX_CYS 주입과 동일 패턴)
            }
            match std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
                Ok(log) => {
                    if let Ok(err) = log.try_clone() {
                        cmd.stderr(err);
                    }
                    cmd.stdout(log);
                }
                Err(_) => {
                    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                }
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    eprintln!("[cysd] office-bridge spawned (127.0.0.1:{port})");
                    let _ = child.wait().await; // 사망 감지 → 아래 백오프 후 루프가 재스폰 판단
                    eprintln!("[cysd] office-bridge exited — 60s 후 재확인");
                }
                Err(e) => eprintln!("[cysd] office-bridge spawn failed: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

/// ★B3: 동봉 runtime python3 절대경로(exe 옆 번들). runtime_bin_dirs(pane 자식과 동일 SOT)에서 python3 실행파일을
/// 찾는다. 없으면 None(호출측이 "python3" 리터럴로 폴백 — PATH 선두주입이 동봉본을 잡거나 시스템 python3).
pub(crate) fn bundled_python3(exe_dir: &std::path::Path) -> Option<String> {
    let names: &[&str] = if cfg!(windows) {
        &["python3.exe", "python.exe"]
    } else {
        &["python3"]
    };
    for d in cys::runtime_bin_dirs(exe_dir) {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// ★P0-7 최종 층위(D1/W5·CI 28780215417 계열): 소켓/파이프 listening 직후 공통 부트 — **양 플랫폼 accept_loop가
/// 반드시 호출한다**. ①이전 실행 잔재 phoenix-embed prune(temp 누수 0) ②콜드부트 auto-restore 1회 발화.
/// state_dir 은 canonical 매핑(crate::state::state_dir — Windows LOCALAPPDATA/cys/<slug>·unix 소켓 부모)으로
/// 계산해 phoenix·스모크와 로그 경로가 일치한다(unix 의 socket_path.parent()는 Windows 파이프엔 부적합).
/// ★과거 `#[cfg(windows)] accept_loop` 에 이 호출이 없어(unix 만 배선) Windows 는 auto-restore 가 발동조차
/// 안 하고(triggered/skipped 라인 전무) phoenix-restore.log 가 빈 파일이던 P0-7 마지막 결함(CI 주입 우회가
/// 가려온 미배선)을 봉인. cfg 무관 단일 함수라 한쪽 누락이 재발하지 않는다(회귀 테스트로 소스 잠금).
fn post_listen_boot(socket_path: &std::path::Path, daemon: &Arc<Daemon>) {
    let state_dir = crate::state::state_dir(socket_path);
    prune_stale_phoenix_embed(&state_dir);
    spawn_auto_restore(&state_dir, socket_path, daemon);
}

/// 콜드부트 auto-restore를 detached 스폰한다(env에 CYS_NO_AUTOSTART=1 — 자식 CLI가 라이벌
/// 데몬을 autostart하는 재귀를 차단). 대기 스레드가 자식을 reap해 좀비 잔존을 막는다.
/// ★W1: PHOENIX_CYS·PATH 주입(§5-1 침묵사 근원 수리) · stdout/stderr 를 null 대신 phoenix-restore.log 로
/// 캡처(P0-5 사후 진단 불가 수리) · exit 계약 처리(5·6=재시도 금지, 그 외 비0=60s 후 1회 재시도).
fn spawn_auto_restore(
    state_dir: &std::path::Path,
    socket_path: &std::path::Path,
    daemon: &std::sync::Arc<Daemon>,
) {
    let opted_out = cys::env_compat("CYS_NO_AUTORESTORE")
        .map(|v| v == "1")
        .unwrap_or(false);
    // exe_dir(데몬 바이너리 디렉터리) — PHOENIX_CYS·PATH 계산 기준.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let current_path = std::env::var("PATH").unwrap_or_default();
    let exe_dir_ref = exe_dir.as_deref().unwrap_or_else(|| std::path::Path::new("."));
    let socket = socket_path.to_string_lossy();
    match decide_auto_restore(&cys::pack::pack_dir(), opted_out, exe_dir_ref, &current_path, &socket) {
        AutoRestore::OptedOut => {
            eprintln!("[cysd] auto-restore skipped (CYS_NO_AUTORESTORE=1)");
        }
        AutoRestore::Ready { program, mut args, env } => {
            // args = [disk_phoenix, "restore", "--auto"]. disk_phoenix 는 B1 폴백 후보.
            let disk_phoenix = std::path::PathBuf::from(args.remove(0));
            let tail = args; // ["restore","--auto"]
            let log_path = state_dir.join("phoenix-restore.log");
            let state_dir = state_dir.to_path_buf();
            let daemon = daemon.clone();
            std::thread::spawn(move || {
                let log_for_panic = log_path.clone();
                // ★P0-5 침묵사 차단(D3/W5·CI 28780215417 실증: auto-restore 스레드가 std/time.rs panic 으로 즉사
                //   → phoenix-restore.log 빈 파일·원인 불가시). 스레드 본문을 guard_restore_panic(catch_unwind)로
                //   감싸 panic 을 삼키지 않고 stderr + phoenix-restore.log 에 **1회 기록**한다 — 무한 재스폰 금지
                //   (재기동은 다음 데몬 부트/schtasks 소관). 이 웨이브가 죽이려는 '스레드 침묵사' 클래스의 구조 수리.
                guard_restore_panic(&log_for_panic, || {
                    // ★B1: 임베드 추출 실행 우선(바이너리=스크립트 동일 커밋 하드보장) → 실패 시 manifest-검증 디스크 폴백.
                    match resolve_phoenix_source(&state_dir, &disk_phoenix, &program, &daemon) {
                        PhoenixResolve::Ready { script, cleanup } => {
                            let mut run_args = vec![script.to_string_lossy().into_owned()];
                            run_args.extend(tail);
                            loop_auto_restore(&daemon, &program, &run_args, &env, &log_path);
                            // temp 누수 0: 추출본은 실행 후 정리(디스크 폴백은 cleanup=None).
                            if let Some(dir) = cleanup {
                                let _ = std::fs::remove_dir_all(&dir);
                            }
                        }
                        PhoenixResolve::Failed(reason) => {
                            eprintln!("[cysd] auto-restore ABORTED — 안전한 phoenix 없음: {reason}");
                            daemon.push_feed_notification(
                                "error",
                                "auto-restore 중단",
                                &format!("안전한 phoenix 실행원 없음(임베드 추출·디스크 폴백 모두 실패): {reason}"),
                                None,
                            );
                        }
                    }
                });
            });
            eprintln!("[cysd] auto-restore triggered (phoenix restore --auto · 임베드 추출 우선)");
        }
    }
}

/// ★B1 phoenix 실행원 해석 결과.
enum PhoenixResolve {
    /// 실행 가능한 phoenix 스크립트. cleanup=Some(dir)면 실행 후 그 임시 디렉터리를 정리한다(추출본).
    Ready {
        script: std::path::PathBuf,
        cleanup: Option<std::path::PathBuf>,
    },
    /// 임베드 추출·디스크 폴백 모두 실패 — auto-restore 중단(사유 보고).
    Failed(String),
}

/// PACK_ALL 에서 phoenix 실행에 필요한 bin/ 트리(javis_phoenix.py + 형제 의존 javis_state_snapshot.py 등)를 추린다.
fn phoenix_embed_files() -> Vec<(&'static str, &'static str)> {
    cys::pack::PACK_ALL
        .iter()
        .copied()
        .filter(|(rel, _)| rel.starts_with("bin/"))
        .collect()
}

/// ★B1①: 임베드 phoenix 트리를 <state>/phoenix-embed/<version>-<uuid>/ 에 추출한다(버전+고유 ID 격리).
/// 추출 실패(공간·권한·noexec)는 Err — 호출측이 디스크 폴백으로 강등한다. 반환=(추출 루트, phoenix 스크립트 경로).
/// ★codex W4 major: 중간 실패(create_dir_all/write) 시 이미 만든 partial root 를 즉시 remove_dir_all(정리 후 Err)
///   — temp 누수 0(다음 부팅 prune 에 의존하지 않는다).
fn extract_phoenix_embed(
    state_dir: &std::path::Path,
) -> std::io::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let version = env!("CARGO_PKG_VERSION");
    let uniq = {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{version}-{}-{nanos}", std::process::id())
    };
    let root = state_dir.join("phoenix-embed").join(uniq);
    let write_all = || -> std::io::Result<()> {
        let mut written = 0u32;
        for (rel, content) in phoenix_embed_files() {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            written += 1;
            // 테스트 seam: root+일부 파일 생성 후 강제 실패 주입(중간 실패 정리 결정론 검증).
            if written == 1 && std::env::var("CYS_PHOENIX_EXTRACT_FAIL").is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "injected mid-extraction failure",
                ));
            }
        }
        Ok(())
    };
    match write_all() {
        Ok(()) => {
            let script = root.join("bin").join("javis_phoenix.py");
            Ok((root, script))
        }
        Err(e) => {
            // partial root 즉시 정리(temp 누수 0). best-effort — 정리 실패해도 원 에러를 반환.
            let _ = std::fs::remove_dir_all(&root);
            Err(e)
        }
    }
}

/// ★B1③: 추출된 phoenix self-test — `<python> <script> --selftest` 가 exit 0 + "selftest ok" 응답이면 통과.
/// 실행성만 확인(데몬·상태 무접촉). 실패=false(호출측이 정리 후 디스크 폴백).
fn phoenix_self_test(python: &str, script: &std::path::Path) -> bool {
    use crate::state::HideConsole;
    // ★SEAL-1: 동봉 python 직스폰 — 팩토리가 PYTHONDONTWRITEBYTECODE 를 얹는다(번들 봉인 보호).
    let out = cys::python_command(python)
        .arg(script)
        .arg("--selftest")
        .env("CYS_NO_AUTOSTART", "1")
        .stdin(std::process::Stdio::null())
        .hide_console()
        .output();
    match out {
        Ok(o) => o.status.success() && String::from_utf8_lossy(&o.stdout).contains("selftest ok"),
        Err(_) => false,
    }
}

/// ★B1②④: phoenix 실행원 해석 — 임베드 추출+self-test 우선, 실패 시 manifest-해시 검증 디스크 폴백.
/// stale 디스크(임베드와 해시 불일치)는 거부+보고(구버전 phoenix 실행 금지). 전 폴백 실패=Failed.
fn resolve_phoenix_source(
    state_dir: &std::path::Path,
    disk_phoenix: &std::path::Path,
    python: &str,
    daemon: &std::sync::Arc<Daemon>,
) -> PhoenixResolve {
    // 1) 임베드 추출 우선.
    match extract_phoenix_embed(state_dir) {
        Ok((root, script)) => {
            if phoenix_self_test(python, &script) {
                return PhoenixResolve::Ready { script, cleanup: Some(root) };
            }
            let _ = std::fs::remove_dir_all(&root); // temp 누수 0(self-test 실패분 즉시 정리)
            eprintln!("[cysd] phoenix 임베드 self-test 실패 — 디스크 폴백 시도");
            daemon.push_feed_notification(
                "warn",
                "phoenix 임베드 self-test 실패",
                "임베드 추출본이 --selftest 를 통과하지 못함 — 디스크 폴백으로 강등(침묵 금지).",
                None,
            );
        }
        Err(e) => {
            eprintln!("[cysd] phoenix 임베드 추출 실패({e}) — 디스크 폴백 시도");
            daemon.push_feed_notification(
                "warn",
                "phoenix 임베드 추출 실패",
                &format!("추출 실패({e}) — manifest-검증 디스크 폴백으로 강등(침묵 금지)."),
                None,
            );
        }
    }
    // 2) 디스크 폴백 — ★codex W4 major: script-only 해시가 아니라 phoenix 실행 closure **전체**(phoenix_embed_files
    //    단일 소스 — 추출과 동일 목록)를 대조한다. javis_phoenix.py 만 일치하고 형제 의존(javis_state_snapshot.py)이
    //    부재/stale 인 디스크 팩이 통과하던 구멍을 막는다. 하나라도 불일치/부재=거부+어느 rel 인지 보고.
    match disk_fallback_verify(disk_phoenix) {
        Ok(()) => {
            eprintln!(
                "[cysd] phoenix 디스크 폴백 채택(전 closure 해시 일치·verified): {}",
                disk_phoenix.display()
            );
            PhoenixResolve::Ready {
                script: disk_phoenix.to_path_buf(),
                cleanup: None,
            }
        }
        Err(reason) => {
            daemon.push_feed_notification(
                "error",
                "phoenix 디스크 폴백 거부(stale/불완전)",
                &format!("디스크 팩 phoenix closure 검증 실패 — 실행 거부(구/불완전 phoenix 부활 금지): {reason}"),
                None,
            );
            PhoenixResolve::Failed(format!("디스크 폴백 closure 검증 실패 — {reason}"))
        }
    }
}

/// ★B1②(codex W4): 디스크 팩 phoenix closure 전체 검증 — phoenix_embed_files(추출과 동일 단일 소스)의
/// 각 rel 이 <pack>/<rel> 로 존재하고 임베드 내용과 해시 일치해야 Ok. 부재/불일치=Err(어느 rel 인지 명시).
/// disk_phoenix = <pack>/bin/javis_phoenix.py → pack_dir = 그 조부모(bin 의 부모).
fn disk_fallback_verify(disk_phoenix: &std::path::Path) -> Result<(), String> {
    let pack_dir = disk_phoenix
        .parent()
        .and_then(|bin| bin.parent())
        .ok_or_else(|| "디스크 phoenix 경로에서 pack_dir 파생 실패".to_string())?;
    let files = phoenix_embed_files();
    if files.is_empty() {
        return Err("임베드 phoenix closure 비었음(빌드 이상)".to_string());
    }
    for (rel, content) in files {
        let path = pack_dir.join(rel);
        match std::fs::read_to_string(&path) {
            Ok(disk) => {
                if cys::pack::content_hash_pub(&disk) != cys::pack::content_hash_pub(content) {
                    return Err(format!("stale(해시 불일치): {rel}"));
                }
            }
            Err(_) => return Err(format!("부재/읽기실패: {rel}")),
        }
    }
    Ok(())
}

/// ★B1: 이전 실행의 잔여 phoenix-embed 디렉터리를 정리한다(크래시로 cleanup 못한 잔재 — temp 누수 방지).
fn prune_stale_phoenix_embed(state_dir: &std::path::Path) {
    let root = state_dir.join("phoenix-embed");
    if let Ok(rd) = std::fs::read_dir(&root) {
        for ent in rd.flatten() {
            let _ = std::fs::remove_dir_all(ent.path());
        }
    }
}

/// ★W1 재시도 지연(codex major test seam): 기본 60000ms. CYS_AUTORESTORE_RETRY_DELAY_MS 로 override —
/// 테스트가 sleep 0 으로 결정론 검증(1차 비0→2차 NOOP·중복 스폰 0, 5/6 무재시도)을 돌리게 한다.
fn autorestore_retry_delay() -> std::time::Duration {
    let ms = std::env::var("CYS_AUTORESTORE_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60_000);
    std::time::Duration::from_millis(ms)
}

/// auto-restore 자식을 실행하고 exit 계약에 따라 처리한다. 비0(단 5·6 제외)은 delay 후 정확히 1회 재시도한다
/// — 재시도의 멱등성은 phoenix 의 lease·liveness 재산정에 맡긴다(수동 복원이 이미 끝났으면 재시도는 NOOP·중복 스폰 0).
fn loop_auto_restore(
    daemon: &std::sync::Arc<Daemon>,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    log_path: &std::path::Path,
) {
    let daemon = daemon.clone();
    let program = program.to_string();
    let args = args.to_vec();
    let env = env.to_vec();
    let log_path = log_path.to_path_buf();
    loop_auto_restore_with(
        |_attempt| run_auto_restore_once(&daemon, &program, &args, &env, &log_path),
        autorestore_retry_delay(),
    );
}

/// ★재시도 결정 루프(test seam · 순수 로직 — 러너·지연 주입). 반환 = 실행 횟수(테스트 단언용).
/// exit 계약: 0=성공 종료 · 5(BREAKER)/6(CORRUPT·identity)=재시도 금지 · 그 외 비0/None=delay 후 1회 재시도.
fn loop_auto_restore_with<F>(mut run: F, retry_delay: std::time::Duration) -> u32
where
    F: FnMut(u32) -> Option<i32>,
{
    let mut attempt = 0u32;
    loop {
        let code = run(attempt);
        attempt += 1;
        match code {
            Some(0) => {
                eprintln!("[cysd] auto-restore finished (exit=0)");
                return attempt;
            }
            Some(5) => {
                eprintln!("[cysd] auto-restore BREAKER_OPEN (exit=5) — 재시도 금지(크래시루프 정지·사람 승인 필요)");
                return attempt;
            }
            Some(6) => {
                eprintln!("[cysd] auto-restore CORRUPT/identity (exit=6) — 재시도 금지(사람 개입 필요)");
                return attempt;
            }
            other => {
                if attempt >= 2 {
                    eprintln!(
                        "[cysd] auto-restore finished (exit={other:?}) — 재시도 소진(1회). phoenix-restore.log 참조"
                    );
                    return attempt;
                }
                eprintln!(
                    "[cysd] auto-restore non-zero (exit={other:?}) — {}ms 후 1회 재시도 (lease/liveness 재산정에 위임)",
                    retry_delay.as_millis()
                );
                std::thread::sleep(retry_delay);
            }
        }
    }
}

/// ★W1 로그 대상 결정(codex major): phoenix-restore.log(primary) → temp_dir 폴백 → 둘 다 실패면 inherit.
/// null 로 떨어뜨리지 않는다 — 파일시스템/경로 실패가 진단 대상인데 그 순간 증거를 소실시키는 게 정확히 W1 관측성
/// 위반이므로, 최악이라도 자식 stdio 를 데몬 stderr 로 inherit 해 증거를 보존한다.
fn open_restore_log(log_path: &std::path::Path) -> Option<std::fs::File> {
    use std::io::Write;
    let open = |p: &std::path::Path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    };
    let f = open(log_path).or_else(|| {
        let tmp = std::env::temp_dir().join("cys-phoenix-restore.log");
        let alt = open(&tmp);
        if alt.is_some() {
            eprintln!(
                "[cysd] auto-restore primary log 실패({}) — temp 폴백 {}",
                log_path.display(),
                tmp.display()
            );
        }
        alt
    });
    if let Some(mut f) = f {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(
            f,
            "\n===== phoenix auto-restore @ epoch={epoch} (pid cysd={}) =====",
            std::process::id()
        );
        Some(f)
    } else {
        eprintln!(
            "[cysd] auto-restore log(primary+temp) 모두 open 실패 — 자식 stdio 를 데몬 stderr 로 inherit(증거 소실 방지)"
        );
        None
    }
}

/// ★P0-5 침묵사 차단(D3/W5): auto-restore 스레드 본문을 catch_unwind 로 감싸 panic 을 삼키지 않는다. panic 시
/// stderr + phoenix-restore.log 에 1회 기록하고 반환(스레드는 자연 종료 — 무한 재스폰 없음). 순수·테스트 가능:
/// 반환 true=정상 완료·false=panic 포착(테스트 단언용).
fn guard_restore_panic<F: FnOnce()>(log_path: &std::path::Path, body: F) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(()) => true,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic payload".to_string());
            eprintln!("[cysd] ★auto-restore 스레드 panic 포착(P0-5 침묵사 차단·재스폰 안 함): {msg}");
            // phoenix-restore.log 에도 남겨 관측성 확보(빈 로그 → panic 기록으로 원인 직결).
            if let Some(mut f) = open_restore_log(log_path) {
                use std::io::Write;
                let _ = writeln!(f, "[cysd] AUTO-RESTORE THREAD PANIC (P0-5 차단·재스폰 안 함): {msg}");
            }
            false
        }
    }
}

/// 자식 1회 실행 — stdout/stderr 를 phoenix-restore.log(폴백 포함)에 append. exit code 반환(None=스폰 실패/대기 실패).
/// ★T6: status()→spawn()+wait() 전환. spawn 직후 무블로킹 최우선으로 pid+start_time 을 확보해,
/// Some(start_time)일 때만 RestoreRootGuard 로 restore_roots 에 등록한다(자식이 살아있는 동안만
/// authoritative 면제 창을 연다·게이트=handlers.rs). 관측 실패(None)면 bounded retry 후에도 None 이면
/// **등록 없이** 진행하고(면제 없음 — phoenix 2회 재시도 경로가 커버) 자식은 반드시 wait/reap 한다(좀비 0).
/// guard 는 함수 종료(정상·early return·panic unwind)에서 Drop 되어 등록을 해제한다. exit 매핑은
/// 기존 status().code() 계약과 동형(0/5/6/비0/None).
fn run_auto_restore_once(
    daemon: &std::sync::Arc<Daemon>,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    log_path: &std::path::Path,
) -> Option<i32> {
    // ★SEAL-1: program 은 동봉 python(decide_auto_restore 가 해석) — 팩토리가
    // PYTHONDONTWRITEBYTECODE 를 얹어 콜드부트 복원이 번들 봉인을 깨지 않게 한다.
    let mut cmd = cys::python_command(program);
    cmd.args(args).env("CYS_NO_AUTOSTART", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::null());
    {
        // Windows: 콜드부트 auto-restore(launch-agent 등)가 수십 초 돌며 콘솔 창을 띄우지 않게.
        use crate::state::HideConsole;
        cmd.hide_console();
    }
    match open_restore_log(log_path) {
        Some(f) => {
            // stderr 는 clone 으로 같은 파일에 합류. clone 실패 시 null 이 아니라 inherit(증거 보존).
            match f.try_clone() {
                Ok(errf) => {
                    cmd.stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::from(errf));
                }
                Err(e) => {
                    eprintln!("[cysd] auto-restore log stderr clone 실패({e}) — stderr inherit 폴백(null 금지)");
                    cmd.stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::inherit());
                }
            }
        }
        None => {
            cmd.stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[cysd] auto-restore spawn failed: {e}");
            return None;
        }
    };
    let pid = child.id();
    // spawn 직후 다른 blocking 없이 최우선으로 start_time 확보(publication race 최소화·C2).
    // bounded retry(3회) — 갓 스폰된 자식이 프로세스표에 반영될 짧은 창을 흡수한다.
    let start_time = {
        let mut st = None;
        for _ in 0..3 {
            if let Some(s) = crate::state::peer_start_time(pid) {
                st = Some(s);
                break;
            }
        }
        st
    };
    // Some(start_time)일 때만 등록 — None 은 restore_roots 에 저장 금지(면제 없음·fail-safe).
    let _guard = start_time.map(|s| crate::state::RestoreRootGuard::new(daemon.clone(), pid, s));
    match child.wait() {
        Ok(s) => Some(s.code().unwrap_or(-1)),
        Err(e) => {
            eprintln!("[cysd] auto-restore wait failed: {e}");
            None
        }
    }
}

/// T1-3: UDS peer pid 조회 — macOS LOCAL_PEERPID, Linux SO_PEERCRED.
#[cfg(unix)]
fn peer_pid(stream: &tokio::net::UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        const SOL_LOCAL: libc::c_int = 0;
        const LOCAL_PEERPID: libc::c_int = 0x002;
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r == 0 && pid > 0 {
            return Some(pid as u32);
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r == 0 && cred.pid > 0 {
            return Some(cred.pid as u32);
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = fd;
        None
    }
}

/// Windows accept_loop가 `connect()` 오류 후 같은 broken 인스턴스에 곧장 재시도하다
/// 100% CPU로 spin하지 않도록 두는 backoff. mio `ConnectNamedPipe`는 정상 대기는
/// WouldBlock(→tokio가 await)으로, 진짜 OS 오류는 즉시 Err로 반환하므로(connecting 플래그도
/// 즉시 해제 → self-throttle 없음), 오류 분기는 ①로그 ②인스턴스 재생성 ③이 짧은 sleep로
/// 회생해야 Unix arm(accept err→다음 await)·tokio 표준 루프(?로 전파)와 대칭이 된다.
/// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로, 정책 값을 모듈 최상위로 빼
///  비-Windows 테스트가 'spin 방지=non-zero backoff' 불변을 박제하게 한다.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_ACCEPT_ERROR_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// Windows named pipe 리스너 풀 크기 — UDS listen backlog 의 대응물. named pipe 엔 backlog 가
/// 없어 '여분 listening 인스턴스 수'가 곧 동시 접속 수용량이다. 1이면 accept→인스턴스 재생성
/// 사이 창(tokio 스케줄링 지연 포함)에 도착한 동시 접속이 전부 ERROR_PIPE_BUSY(os error 231,
/// "모든 파이프 인스턴스가 사용 중")로 튕긴다 — 멀티 노드(master·cso·worker·reviewer 동시 RPC)
/// + GUI 기동 fan-out(daemon_status·pane attach·event forwarder)에서 상시 재현
/// (2026-07-10 Windows 실사고: GUI "startup failed … os error 231"). 클라이언트 busy-retry 와
/// 이중 방어. (Windows arm 은 이 호스트에서 컴파일/실행 불가하므로, 정책 값을 모듈 최상위로 빼
///  비-Windows 테스트가 '풀 ≥ 2' 불변을 박제하게 한다 — PIPE_ACCEPT_ERROR_BACKOFF 와 같은 방식.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_LISTENER_POOL: usize = 8;

/// owner-only DACL의 SDDL: D:P=보호된(상속차단) DACL, FA=full access를
/// OW(OWNER_RIGHTS=creator)·SY(SYSTEM)·BA(BUILTIN\Administrators)에게만 부여.
/// WD(Everyone)·AU(Authenticated Users) 같은 광역 SID가 없어 같은 머신의 임의 사용자를 배제한다.
/// (cfg(windows) 밖에서도 회귀 테스트가 참조할 수 있게 모듈 최상위 const로 둔다.
///  비-Windows 비-test 빌드에서는 실사용처가 없으므로 dead_code를 명시 허용한다.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_SDDL_OWNER_ONLY: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)";

/// Windows named pipe 보안 디스크립터: 소유자(creator)·SYSTEM·Administrators에게만
/// full access를 허용하는 owner-only DACL(PIPE_SDDL_OWNER_ONLY)을 SECURITY_ATTRIBUTES에 싣는다.
/// UDS 0o700 dir + 0o600 소켓의 단일-UID 봉인과 대칭 — 같은 머신의 임의 로컬 사용자가
/// 인증 없는 제어 채널(send_text·send_key·ledger.kill)에 접근하는 권한 우회를 차단한다.
/// 반환된 PSECURITY_DESCRIPTOR는 LocalFree로 해제해야 하므로, RAII 가드로 SA와 함께 수명을 묶는다.
#[cfg(windows)]
struct OwnerOnlySecurity {
    sa: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    psd: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
}

#[cfg(windows)]
impl OwnerOnlySecurity {
    fn new() -> Option<Self> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        // 와이드 널종단 SDDL 문자열
        let sddl: Vec<u16> = PIPE_SDDL_OWNER_ONLY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut psd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || psd.is_null() {
            return None;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd,
            bInheritHandle: 0,
        };
        Some(Self { sa, psd })
    }

    /// create_with_security_attributes_raw에 넘길 *mut SECURITY_ATTRIBUTES (가드 수명 동안 유효).
    fn as_ptr(&self) -> *mut std::ffi::c_void {
        &self.sa as *const _ as *mut std::ffi::c_void
    }
}

#[cfg(windows)]
impl Drop for OwnerOnlySecurity {
    fn drop(&mut self) {
        // ConvertString…가 LocalAlloc로 잡은 SD를 해제 (가드가 데몬 수명 동안 살아있으므로
        // 실무상 프로세스 종료 시점에만 호출되나, 누수 방지를 위해 명시 해제).
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(self.psd as *mut _);
        }
    }
}

/// named pipe listening 인스턴스 1개 생성. owner-only DACL 은 인스턴스마다 새로 변환한다 —
/// 커널이 CreateNamedPipe 시점에 SD 를 파이프 객체로 복사하므로 SECURITY_ATTRIBUTES 는 이 호출
/// 동안만 살아있으면 되고(호출 후 drop 안전), 가드를 태스크 간 공유하지 않아 리스너 풀의
/// spawn(Send 경계)과 충돌하지 않는다. SDDL 변환 실패(이론상 거의 없음)면 null 폴백 + 경고
/// — 기존 accept_loop 폴백 정책 그대로.
#[cfg(windows)]
fn create_pipe_instance(
    pipe_name: &str,
    first: bool,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let security = OwnerOnlySecurity::new();
    if security.is_none() {
        eprintln!(
            "warning: failed to build owner-only pipe security descriptor; \
             falling back to default DACL (any local user may connect)"
        );
    }
    let sa_ptr = security
        .as_ref()
        .map(|s| s.as_ptr())
        .unwrap_or(std::ptr::null_mut());
    // Safety: sa_ptr는 null이거나 `security` 가드가 소유한 유효한 SECURITY_ATTRIBUTES를 가리키며,
    // 그 가드는 이 함수 끝까지 살아있어 파이프 생성 호출보다 오래 산다.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(pipe_name, sa_ptr)
    }
}

/// listening 인스턴스 재생성 — 실패 시 backoff 후 무한 재시도(리스너 태스크 침묵사 방지).
/// 과거 `.expect()` panic 은 메인 accept 태스크에선 데몬 전사(fail-fast·Task Scheduler 재기동)
/// 였지만, 풀의 spawn 태스크에선 tokio 가 panic 을 삼켜 리스너만 조용히 줄어드는 최악
/// (전 리스너 소진 = 무증상 접속 불능)이 된다 — 로그 + 재시도가 정직하다.
#[cfg(windows)]
async fn recreate_pipe_instance(
    pipe_name: &str,
) -> tokio::net::windows::named_pipe::NamedPipeServer {
    loop {
        match create_pipe_instance(pipe_name, false) {
            Ok(s) => return s,
            Err(e) => {
                eprintln!("recreate pipe {pipe_name} failed: {e} — retrying");
                tokio::time::sleep(PIPE_ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}

/// 리스너 풀의 태스크 1개 — 자기 listening 인스턴스로 accept 루프를 돈다.
#[cfg(windows)]
async fn pipe_listener(
    daemon: Arc<Daemon>,
    pipe_name: String,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
) {
    loop {
        match server.connect().await {
            Ok(()) => {
                // 접속 완료된 클라이언트를 먼저 서빙한다 — 재생성(recreate)을 앞에 두면 재생성이
                // 실패를 반복하는 비정상 상태에서 '이미 accept 된' 클라이언트까지 무기한 기아가
                // 된다(liveness 역전). 재생성 지연으로 listening 정원이 잠깐 N-1이 되는 것은
                // 나머지 리스너 + 클라이언트 busy-retry 가 흡수한다.
                let connected = server;
                // 발신자 신원: 커널이 보증하는 named pipe 클라이언트 pid (UDS peer_pid와 대칭).
                // 박는 이유: claim_role·surface.close·status.set 등은 발신 신원이 None이면 무조건
                // 거부하므로, 미구현(None)이면 Windows에서 자기 surface 자가-claim('cys claim-role
                // master' 등 launch-agent 밖 직접 기동 노드)이 영영 막힌다. boxing 전에 조회한다.
                let caller_pid = peer_pid(&connected);
                let handler_daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    handle_connection(handler_daemon, Box::new(connected) as Stream, caller_pid)
                        .await;
                });
                server = recreate_pipe_instance(&pipe_name).await;
            }
            Err(e) => {
                // connect()가 즉시 Err를 반환하면(broken 핸들 등) 같은 인스턴스에 곧장
                // 재시도해도 같은 Err가 무한 반복돼 100% CPU spin이 된다(mio가 connecting
                // 플래그를 즉시 해제해 self-throttle도 없음). Unix arm(accept err→다음 await)·
                // tokio 표준 루프(?로 전파)와 대칭이 되도록: ①로그 ②인스턴스 재생성 ③짧은 backoff.
                eprintln!("accept error: {e}");
                server = recreate_pipe_instance(&pipe_name).await;
                tokio::time::sleep(PIPE_ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}

#[cfg(windows)]
async fn accept_loop(
    daemon: Arc<Daemon>,
    socket_path: &std::path::Path,
    first: tokio::net::windows::named_pipe::NamedPipeServer,
) {
    let pipe_name = socket_path.to_string_lossy().into_owned();
    // ★W1-c: 첫 인스턴스(= 데몬 싱글턴 가드)는 main()에서 부트 부수효과보다 먼저 조기 선점해 넘겨받는다.
    // 여기서 다시 만들지 않고 그대로 리스너 풀에 편입한다(probe-후-close-재open 레이스 제거·경쟁 패자는
    // main()의 선점 실패 지점에서 이미 즉사).
    // ★P0-7 최종 층위(D1/W5): 파이프 listening 직후 공통 부트 — unix accept_loop 와 **동일 함수**(prune +
    //   콜드부트 auto-restore). 과거 이 호출이 Windows 에만 빠져 auto-restore 가 발동조차 안 하고 phoenix-restore.log
    //   가 빈 파일이던 결함(CI 실경로 스모크 ⑧)을 봉인. state_dir 은 함수 내부 canonical 매핑(Windows 슬러그).
    post_listen_boot(socket_path, &daemon);
    // ★리스너 풀(PIPE_LISTENER_POOL): listening 인스턴스 N개를 병렬 대기 — 단일 인스턴스의
    // accept→재생성 사이 창에서 동시 접속이 ERROR_PIPE_BUSY(231)로 튕기던 결함 봉인(상수 주석 참조).
    let mut first = Some(first);
    let mut tasks = Vec::new();
    for _ in 0..PIPE_LISTENER_POOL {
        let server = match first.take() {
            Some(s) => s,
            None => recreate_pipe_instance(&pipe_name).await,
        };
        tasks.push(tokio::spawn(pipe_listener(
            Arc::clone(&daemon),
            pipe_name.clone(),
            server,
        )));
    }
    // accept_loop 는 반환하지 않는 계약(unix arm 대칭) — 리스너 태스크들을 영구 대기한다.
    for t in tasks {
        let _ = t.await;
    }
}

/// Windows named pipe 클라이언트 pid 조회 — UDS peer_pid(macOS LOCAL_PEERPID/Linux SO_PEERCRED)와
/// 대칭. GetNamedPipeClientProcessId는 서버 측 핸들에서 연결된 클라이언트 프로세스 id를 돌려준다.
/// 실패(0 반환 또는 pid 0)면 None — 호출부는 UDS와 동일하게 익명 발신으로 처리한다.
#[cfg(windows)]
fn peer_pid(pipe: &tokio::net::windows::named_pipe::NamedPipeServer) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    let mut pid: u32 = 0;
    let ok = unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
            pipe.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
            &mut pid,
        )
    };
    if ok != 0 && pid != 0 {
        Some(pid)
    } else {
        None
    }
}

/// 개행 없는 무한 스트림이 데몬 메모리를 잠식하지 못하게 줄 길이 상한을 둔 line reader.
async fn next_line_capped<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
    cap: usize,
) -> std::io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&buf).into_owned())
            });
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..pos]);
            r.consume(pos + 1);
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let n = available.len();
        buf.extend_from_slice(available);
        r.consume(n);
        if buf.len() > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request line too long",
            ));
        }
    }
}

const MAX_REQUEST_LINE: usize = 10 * 1024 * 1024; // 지침 주입(수백 KB)에 충분한 10MB

/// ★U-6(서버측) 연결 후 **첫 요청 줄**이 도착하기까지의 상한(초).
///
/// 고친 것: `next_line_capped` 는 줄 길이만 유계이고 **시간은 무계**였다. 연결만 잡고 한 줄도
/// 보내지 않는 클라이언트가 연결 태스크를 영구 점유한다 — 태스크·소켓 핸들·버퍼가 회수되지
/// 않는 누수이고, 이것은 클라이언트측 상한(cys.rs `RpcDeadline`)이 막을 수 없는 반대 방향의
/// wedge다.
///
/// ★근거 정정(P1-4 · 리뷰어 2인 독립 반박 · 코드 재확인 완료): 종전 주석은 "Windows 는 named
/// pipe 인스턴스 풀이 8(`PIPE_LISTENER_POOL`)이라 그런 클라이언트 8개면 데몬이 전원에게
/// 무응답"이라고 적었다. **틀렸다.** `pipe_listener` 는 accept 직후 클라이언트를
/// `tokio::spawn` 으로 떼어내고 **곧바로** listening 인스턴스를 재생성한다
/// (`server = recreate_pipe_instance(&pipe_name).await` — 이 파일의 `pipe_listener`).
/// 그러므로 침묵 클라이언트는 listening 슬롯을 점유하지 않고, `PIPE_LISTENER_POOL` 은
/// "accept→재생성 창의 동시 접속 흡수량"이지 동시 연결 수용량이 아니다.
/// 실제 상한은 **파이프 인스턴스 총수**다: `create_pipe_instance` 는 `ServerOptions::new()` 기본값을
/// 쓰고 그 기본 `max_instances` 는 `PIPE_UNLIMITED_INSTANCES` = **255**
/// (tokio 1.52.3 `net/windows/named_pipe.rs` `ServerOptions::new` · windows-sys
/// `PIPE_UNLIMITED_INSTANCES: u32 = 255`). 즉 무응답까지 필요한 침묵 클라이언트 수는 8이 아니라
/// **연결분 + listening 8이 255에 닿을 때**로, 종전 주석의 약 30배다.
/// ∴ 이 상한의 실효는 "인스턴스 8개 고갈 방지"가 아니라 **누수 태스크·핸들의 회수**이며,
/// 그 회수가 255 고갈이라는 먼 한계선까지의 거리를 유지시킨다. 값 60초는 그대로 둔다 —
/// 근거 문장만 실제 구조로 바꾼 것이고 판정 축·거동은 무변이다.
///
/// ★상한을 **첫 줄에만** 거는 이유(확립된 연결은 건드리지 않는다): GUI(src-tauri `RPC_POOL`)는
/// 소켓별 **영속 연결**을 재사용한다. 유휴 중 서버가 끊으면 다음 RPC 가 `rpc_full` 의
/// `AfterSend`(= 재시도 금지 분기)로 떨어져 사용자에게 오류로 보인다. 유휴 끊기는 그래서 금지다.
/// 첫 줄 상한은 그 경로에 닿지 않는다 — 풀은 연결 직후 곧바로 요청을 쓴다.
/// 판정 축을 옮긴 것이 아니라(줄 길이 상한 그대로), 시간 축을 **없던 곳에 새로** 둔 것이다.
///
/// 값 60초: 연결 후 60초 동안 한 줄도 못 보내는 클라이언트는 정상 상태가 아니다. 정상 클라이언트는
/// connect 직후 write 하므로(cys `rpc_roundtrip` · GUI `rpc_once` · deadman `probe_holder`) 실측
/// 여유가 3자릿수 배다. 생존 프로브(connect 후 즉시 drop)는 EOF 로 먼저 빠져나가 무관하다.
const FIRST_LINE_IDLE_SECS: u64 = 60;

/// 노브가 받아들이는 **상한**(초 · 1일). 이보다 큰 선언은 여기로 접는다.
///
/// ★왜 상한이 필요한가(실사고 클래스): 아래 판정부의 결과는 곧바로
/// `tokio::time::Instant::now() + cap` 이 된다. std `Instant::add` 는 오버플로에서
/// **패닉**하므로, `CYS_CONN_FIRST_LINE_SECS` 에 대략 9.2e18 이상이 들어오면 **모든 연결
/// 핸들러 태스크가 패닉**한다 — 데몬은 accept 만 하고 전 RPC 가 불통이 된다(살아 있는 척하는
/// 데몬이 가장 나쁜 상태다). 음수·비숫자는 이미 안전하다(parse 실패 → 기본값 · 진리표가 박제).
/// 뚫린 것은 **거대 양수** 한 축뿐이었다.
/// 값 1일: "첫 줄을 하루 동안 안 보내는 클라이언트"는 어떤 운용에서도 정상이 아니고,
/// 상한 해제를 원하면 계약대로 `0`(= 무한 대기)을 쓰면 된다 — 노브의 표현력이 줄지 않는다.
const FIRST_LINE_IDLE_MAX_SECS: u64 = 86_400;

/// 롤백 스위치 — `CYS_CONN_FIRST_LINE_SECS=0` 이면 첫 줄 상한 해제(개정 전 무한 대기 거동),
/// 양수면 그 값(초 · [`FIRST_LINE_IDLE_MAX_SECS`] 로 클램프). 코드 revert 없이 무력화 가능해야
/// 한다는 단위 계약의 집행부다.
fn first_line_idle_timeout() -> Option<std::time::Duration> {
    parse_first_line_cap(cys::env_compat("CYS_CONN_FIRST_LINE_SECS").as_deref())
}

/// 순수 판정부 — env 를 인자로 받아 테스트가 프로세스 전역 env 를 흔들지 않게 한다.
///
/// **불변식**: 반환값은 언제나 `Instant` 에 더할 수 있다(오버플로 패닉 도달 불가).
/// 진리표가 그 불변식을 전 축에서 실측한다.
fn parse_first_line_cap(raw: Option<&str>) -> Option<std::time::Duration> {
    match raw.and_then(|v| v.trim().parse::<u64>().ok()) {
        Some(0) => None,
        Some(v) => Some(std::time::Duration::from_secs(v.min(FIRST_LINE_IDLE_MAX_SECS))),
        None => Some(std::time::Duration::from_secs(FIRST_LINE_IDLE_SECS)),
    }
}

async fn handle_connection(daemon: Arc<Daemon>, stream: Stream, caller_pid: Option<u32>) {
    handle_connection_capped(daemon, stream, caller_pid, first_line_idle_timeout()).await
}

/// `handle_connection` 의 본체 — 첫 줄 상한을 인자로 받는다(테스트가 짧은 상한으로 실제 경로를
/// 그대로 돌릴 수 있게. env 를 흔들면 병렬 테스트가 서로를 오염시킨다).
async fn handle_connection_capped(
    daemon: Arc<Daemon>,
    stream: Stream,
    caller_pid: Option<u32>,
    first_line_cap: Option<std::time::Duration>,
) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut awaiting_first_line = true;
    // ★P1-3: 첫 줄 상한은 **절대 데드라인**이다(매 반복 재장전하는 상대 상한이 아니다).
    //   상대 상한이면 빈 줄을 상한보다 짧은 주기로 흘리는 것만으로 상한이 무한히 밀린다
    //   — 그건 이 상한이 막으려던 바로 그 wedge 다. 한 번만 계산해 `timeout_at` 에 넘긴다.
    let first_line_deadline = first_line_cap.map(|cap| tokio::time::Instant::now() + cap);

    loop {
        // 첫 줄만 시간 유계(위 상수 주석의 근거). 그 뒤 줄들은 종전대로 무계 — 영속 연결을
        // 쥔 정상 클라이언트를 유휴만으로 끊지 않는다.
        let read = match (awaiting_first_line, first_line_deadline) {
            (true, Some(at)) => {
                match tokio::time::timeout_at(at, next_line_capped(&mut reader, MAX_REQUEST_LINE))
                    .await
                {
                    Ok(r) => r,
                    // 무언의 클라이언트 — 연결을 회수한다(조용히: 요청이 없었으므로 돌려줄
                    // 응답 프레임도 없다. 상대는 소켓 EOF 로 알게 된다).
                    Err(_) => return,
                }
            }
            _ => next_line_capped(&mut reader, MAX_REQUEST_LINE).await,
        };
        let Ok(Some(line)) = read else {
            return; // 종전 `while let Ok(Some(line))` 과 동일 종료 조건(EOF·과대 줄·I/O 오류)
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            // ★P1-3(순서가 판정이다): **빈 줄은 첫 줄로 계상하지 않는다.**
            //   종전에는 `awaiting_first_line = false` 가 이 검사보다 **앞**에 있어, 클라이언트가
            //   개행 1바이트(`"\n"`)만 보내고 침묵하면 상한이 통째로 풀린 채 연결이 영구
            //   잔존했다 — 1바이트로 무장 해제되는 상한은 없는 것과 같다. 빈 줄은 요청이
            //   아니므로(파싱조차 하지 않는다) '첫 요청 줄이 도착했다'의 근거가 될 수 없다.
            continue;
        }
        // 여기까지 온 줄만이 **요청**이다 — 이 시점에 상한을 놓는다.
        awaiting_first_line = false;
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp =
                    cys::err_response(&serde_json::Value::Null, "parse_error", &e.to_string());
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
                continue;
            }
        };

        match handlers::dispatch(&daemon, req, caller_pid) {
            Reply::Single(resp) => {
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
            Reply::EventStream {
                ack,
                after_seq,
                names,
                categories,
            } => {
                run_event_stream(&daemon, &mut write_half, ack, after_seq, names, categories).await;
                return;
            }
            Reply::Attach { ack, surface_id } => {
                run_attach(&daemon, &mut write_half, ack, surface_id).await;
                return;
            }
            Reply::FeedWait {
                id,
                request_id,
                rx,
                timeout_secs,
            } => {
                // T4-15: pause 중에는 카운트다운 동결 — kill-switch가 대기 중인 워커들을
                // timeout-deny로 우수수 떨어뜨리지 않는다 (resume 후 잔여 시간부터 재개).
                let mut rx = rx;
                let mut remaining = timeout_secs;
                let outcome: Option<String> = loop {
                    tokio::select! {
                        r = &mut rx => break r.ok(),
                        // 클라이언트 연결 끊김 감지: 대기 중에는 응답을 아직 쓰기 전이라
                        // events.stream·attach의 write 실패 안전망이 닿지 않는다. read half를
                        // 함께 감시해, 워커가 응답 전에 끊으면(EOF/에러) 즉시 정리하고 빠져나간다.
                        // 없으면 끊긴 워커의 waiter·연결 태스크가 timeout(최대 3600초)까지,
                        // pause 중에는 remaining이 동결돼 resume까지 무기한 잔존한다.
                        read = reader.fill_buf() => match read {
                            // EOF(빈 슬라이스) = 끊김. 비어있지 않은 바이트는 대기 중 추가 전송으로
                            // 프로토콜 위반이라 연결을 신뢰할 수 없다 — 셋 다 끊김으로 정리.
                            Ok([]) | Ok([_, ..]) | Err(_) => break None,
                        },
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                            if !daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
                                if remaining <= 1 { break None; }
                                remaining -= 1;
                            }
                        }
                    }
                };
                let resp = match outcome {
                    Some(decision) => cys::ok_response(
                        &id,
                        json!({"request_id": request_id, "status": "resolved", "decision": decision}),
                    ),
                    None => {
                        // Timeout or dropped: mark the item and tell the caller.
                        daemon.feed_waiters.lock().unwrap().remove(&request_id);
                        let snapshot = {
                            let mut items = daemon.feed_items.lock().unwrap();
                            items
                                .iter_mut()
                                .find(|i| i.request_id == request_id)
                                .filter(|i| i.status == "pending")
                                .map(|item| {
                                    item.status = "timeout".into();
                                    item.resolved_at = Some(crate::state::now_epoch());
                                    item.clone()
                                })
                        };
                        if let Some(s) = &snapshot {
                            daemon.persist_feed_item(s);
                            daemon.bus.publish(
                                "feed.item.timeout",
                                "feed",
                                None,
                                json!({"request_id": request_id}),
                            );
                            cys::ok_response(
                                &id,
                                json!({"request_id": request_id, "status": "timeout", "decision": null}),
                            )
                        } else {
                            // 동시 feed.reply가 이미 종결 — 승인 결정을 삼키고 timeout으로
                            // 오보하는 대신 실제 결정을 돌려준다 (모순 이벤트도 미발행)
                            let decision = daemon
                                .feed_items
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|i| i.request_id == request_id)
                                .and_then(|i| i.decision.clone());
                            match decision {
                                Some(d) => cys::ok_response(
                                    &id,
                                    json!({"request_id": request_id, "status": "resolved", "decision": d}),
                                ),
                                None => cys::ok_response(
                                    &id,
                                    json!({"request_id": request_id, "status": "timeout", "decision": null}),
                                ),
                            }
                        }
                    }
                };
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
            Reply::WaitFor {
                id,
                surface_id,
                pattern,
                timeout_secs,
                since_line,
            } => {
                // T3-14 완료 대기: 데몬 내부 폴링(토큰 비용 0) — plain-line 마커 규약 전제.
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_secs(timeout_secs);
                let mut cursor = since_line;
                let resp = loop {
                    let Some(surface) = daemon.get_surface(surface_id) else {
                        break cys::err_response(
                            &id,
                            "not_found",
                            &format!("surface {surface_id} closed"),
                        );
                    };
                    let (lines, start) = {
                        // ★레이스 차단: scrollback 락을 먼저 잡고 그 안에서 line_count를 읽는다
                        // (writer가 push·fetch_add를 같은 락 아래 수행 — total/sb.len 일관 관측).
                        let sb = surface.scrollback.lock().unwrap_or_else(|e| e.into_inner());
                        let total = surface
                            .line_count
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let oldest = total.saturating_sub(sb.len() as u64);
                        let start = cursor.max(oldest);
                        let skip = (start - oldest) as usize;
                        let lines: Vec<String> = sb.iter().skip(skip).cloned().collect();
                        (lines, start)
                    };
                    let mut matched = None;
                    for (i, line) in lines.iter().enumerate() {
                        if pattern.is_match(line) {
                            matched = Some((start + i as u64, line.clone()));
                            break;
                        }
                    }
                    cursor = start + lines.len() as u64;
                    if let Some((line_no, line)) = matched {
                        break cys::ok_response(
                            &id,
                            json!({"matched": true, "line": line, "line_no": line_no,
                                   "next_cursor": line_no + 1}),
                        );
                    }
                    if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
                        break cys::ok_response(
                            &id,
                            json!({"matched": false, "reason": "surface_exited",
                                   "next_cursor": cursor}),
                        );
                    }
                    if std::time::Instant::now() >= deadline {
                        break cys::ok_response(
                            &id,
                            json!({"matched": false, "reason": "timeout",
                                   "next_cursor": cursor}),
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                };
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// T1-6: cys↔cysd ABI producer 자기검증 경계. 응답 `Value`를 `cys::wire::frame_response`로
/// 통과시켜 round-trip 동일성(선언==실제 직렬화)을 검증하고 `_flen`/`_pv`를 additive하게
/// 부착한다(top-level `ok`/`result`는 보존 → 구 디코더 호환). 위반은 T1-3 `Severity`로
/// 사상해 fail-loud 기록한다(Drift/LenMismatch=Critical 격리, VersionSkew=Recoverable).
/// 검증 실패가 응답 자체를 삼켜 클라이언트를 무기한 대기시키지 않도록, 기록 후 legacy 직렬화로
/// 폴백해 한 줄은 항상 내보낸다(가용성 보존 — 격리 판정은 Severity 로그가 담당).
fn abi_severity(e: &cys::wire::AbiError) -> severity::Severity {
    match e {
        cys::wire::AbiError::Drift | cys::wire::AbiError::LenMismatch => severity::Severity::Critical,
        cys::wire::AbiError::VersionSkew { .. } => severity::Severity::Recoverable,
    }
}

async fn write_line<W: AsyncWrite + Unpin>(
    w: &mut W,
    value: &serde_json::Value,
) -> std::io::Result<()> {
    // T4-5A(==T5-6 strand-3, ONE guard): 단일 RPC 응답 바이트 상한. cap 초과 시 fail-loud
    // 트렁케이트 sentinel로 치환(컨텍스트/메모리 폭주 차단). 직교 가드 — watchdog와 별개 책임.
    let capped = cys::wire::cap_response(value);
    let value: &serde_json::Value = capped.as_ref().unwrap_or(value);
    let line = match cys::wire::frame_response(value) {
        Ok(framed) => framed,
        Err(e) => {
            let sev = abi_severity(&e);
            eprintln!(
                "[cysd] ABI producer self-verify {} ({:?}) — falling back to legacy serialization",
                sev.as_str(),
                e
            );
            let mut body = serde_json::to_string(value).unwrap_or_default();
            body.push('\n');
            body
        }
    };
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

/// Push channel: replay missed events, then forward live events until the client disconnects.
async fn run_event_stream<W: AsyncWrite + Unpin>(
    daemon: &Arc<Daemon>,
    w: &mut W,
    ack: serde_json::Value,
    after_seq: Option<u64>,
    names: Vec<String>,
    categories: Vec<String>,
) {
    // Subscribe BEFORE replay so no events fall into the gap.
    let mut rx = daemon.bus.subscribe();
    // dispatch 시점이 아닌 구독 직후의 최신 seq로 갱신 — 클라이언트 커서 시드 정확화
    let mut ack = ack;
    let live_latest = daemon.bus.latest_seq();
    ack["latest_seq"] = json!(live_latest);
    // (1)-sync: resume 블록도 구독 직후 최신값으로 동기 — dispatch 시점 값과 어긋나지 않게
    if ack.get("resume").is_some() {
        ack["resume"]["latest_seq"] = json!(live_latest);
        ack["resume"]["next_seq"] = json!(live_latest + 1);
    }
    if write_line(w, &ack).await.is_err() {
        return;
    }
    let mut last_seq = after_seq.unwrap_or(0);
    if let Some(after) = after_seq {
        // 갭 신호: 커서 이후 일부 이벤트가 ring에서 밀려나 재생 불가하면 무음 유실 대신 알린다
        let (oldest, latest) = daemon.bus.replay_bounds();
        let gap_until = oldest.map(|o| o.saturating_sub(1)).unwrap_or(latest);
        if gap_until > after {
            let warn = json!({"type": "error", "ok": false,
                "error": {"code": "replay_gap",
                    "message": format!("events {}..={} no longer available (ring evicted or daemon restarted)", after + 1, gap_until)}});
            if write_line(w, &warn).await.is_err() {
                return;
            }
        }
        for event in daemon.bus.replay_after(after) {
            last_seq = event["seq"].as_u64().unwrap_or(last_seq);
            if events::event_matches(&event, &names, &categories)
                && write_line(w, &event).await.is_err()
            {
                return;
            }
        }
    }
    // (2b) live 루프: 15s heartbeat 타이머와 함께 select! — 이벤트 무발생 구간에서도
    // half-open 소켓을 조기 감지·재연결 유도. 패턴은 run_attach(아래)의 select! 동일.
    let mut hb = tokio::time::interval(std::time::Duration::from_secs(15));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    hb.tick().await; // 첫 tick은 즉시 발화 — 소비해 15s 후부터 heartbeat
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(event) => {
                    let seq = event["seq"].as_u64().unwrap_or(0);
                    if seq <= last_seq {
                        continue; // already replayed
                    }
                    last_seq = seq; // 중복 차단 커서 전진(원본 누락 — 의도 명확화, 동작 동일)
                    if events::event_matches(&event, &names, &categories)
                        && write_line(w, &event).await.is_err()
                    {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let warn = json!({"type": "error", "ok": false,
                        "error": {"code": "slow_consumer", "message": format!("dropped {n} events")}});
                    let _ = write_line(w, &warn).await;
                    return; // (2a) 종료해 클라이언트가 last_seq부터 재replay로 갭을 메우게 강제
                }
                Err(_) => return,
            },
            _ = hb.tick() => {
                let beat = json!({"type": "heartbeat", "latest_seq": daemon.bus.latest_seq()});
                if write_line(w, &beat).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Raw PTY output mirror: ack line (JSON), then raw bytes as they arrive.
async fn run_attach<W: AsyncWrite + Unpin>(
    daemon: &Arc<Daemon>,
    w: &mut W,
    ack: serde_json::Value,
    surface_id: u64,
) {
    let Some(surface) = daemon.get_surface(surface_id) else {
        // dispatch 검사와 재조회 사이에 surface가 닫힌 경우 — 무응답 종료 대신 에러를 알린다
        let err = json!({"type": "ack", "ok": false,
            "error": {"code": "not_found", "message": format!("surface {surface_id} closed")}});
        let _ = write_line(w, &err).await;
        return;
    };
    // parser 락 아래에서 구독+스냅샷 — 그 사이 도착한 청크가 스냅샷과 live 양쪽에
    // 중복 배달되는 창을 닫는다 (reader 스레드는 parser 락에서 직렬화됨)
    let (mut rx, snapshot) = {
        let parser = surface.parser.lock().unwrap_or_else(|e| e.into_inner());
        let rx = surface.out_tx.subscribe();
        (rx, parser.screen().contents_formatted())
    };
    if write_line(w, &ack).await.is_err() {
        return;
    }
    // Send a formatted (color/cursor-accurate) redraw of the current screen first.
    if !snapshot.is_empty() && w.write_all(&snapshot).await.is_err() {
        return;
    }
    loop {
        // out_tx Sender는 Surface 구조체가 소유라 자력 종료(셸 exit) 후에도 채널이 닫히지
        // 않는다 — exited 플래그를 주기 점검해 스트림을 끝내야 클라이언트가 EOF를 받는다.
        tokio::select! {
            r = rx.recv() => match r {
                Ok(chunk) => {
                    if w.write_all(&chunk).await.is_err() || w.flush().await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod startup_lock_retry_tests {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use std::time::Duration;

    #[test]
    fn default_schedule_exceeds_one_second_and_backs_off() {
        let s = schedule_for_budget(1550);
        let ms: Vec<u64> = s.iter().map(|d| d.as_millis() as u64).collect();
        assert_eq!(ms, vec![50, 100, 200, 400, 800], "지수 백오프");
        let total: u64 = ms.iter().sum();
        assert!(
            total >= 1000,
            "총 예산 ≥1s여야 doctor의 순간 락 보유를 흡수한다(실제 {total}ms)"
        );
    }

    #[test]
    fn budget_knob_is_injectable_for_deterministic_tests() {
        assert!(
            schedule_for_budget(0).is_empty(),
            "예산 0 = 재시도 없음(구 동작 재현용)"
        );
        let s = schedule_for_budget(120);
        assert_eq!(
            s.iter().map(|d| d.as_millis() as u64).sum::<u64>(),
            120,
            "예산을 넘지 않는다"
        );
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for _ in 0..200 {
            let d = jittered(Duration::from_millis(100));
            assert!(
                (80..=120).contains(&(d.as_millis() as u64)),
                "±20% 밖: {d:?}"
            );
        }
        assert_eq!(jittered(Duration::from_millis(1)).as_millis(), 1, "미세값 무변");
    }

    #[test]
    fn retry_loop_wins_the_lock_a_doctor_briefly_held() {
        // ★결정론: 벽시계 경합이 아니라 "doctor 보유 구간 < 재시도 예산"을 노브로 고정해 재현한다.
        // 재시도가 없으면 이 시나리오에서 부팅 데몬은 dead-holder-reclaim-failed로 오사유 exit(1)한다.
        let d = std::env::temp_dir().join(format!(
            "cysd-lockretry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        let lock = d.join("cys.lock");
        let doctor = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(doctor.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "doctor가 락을 쥔 상태 모사"
        );
        let hold_ms = 300u64; // doctor 진단 2건 연속 실행의 상한을 넉넉히 상회하는 보유 구간.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(hold_ms));
            drop(doctor);
        });

        let booting = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .unwrap();
        let try_flock = |f: &std::fs::File| unsafe {
            libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0
        };
        assert!(!try_flock(&booting), "첫 시도는 실패(경합 재현)");
        let mut won = false;
        for backoff in schedule_for_budget(1550) {
            std::thread::sleep(jittered(backoff));
            if try_flock(&booting) {
                won = true;
                break;
            }
        }
        assert!(won, "재시도 예산(1550ms) 안에서 {hold_ms}ms 보유를 흡수해 승리해야 한다");
        std::fs::remove_dir_all(&d).ok();
    }
}

#[cfg(test)]
mod env_scrub_tests {
    /// 회귀 박제: claude 세션 안에서 spawn된 데몬이 세션 정체성 env를 보존하면 PTY 자식
    /// claude가 child-session으로 강등돼 트랜스크립트 미영속(복원·recall·T5 전부 파괴).
    /// scrub은 누설 변수만 제거하고 무관 변수는 보존해야 한다.
    #[test]
    fn scrub_removes_leaky_session_vars_only() {
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "parent-session");
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("CYS_SCRUB_TEST_KEEP", "yes"); // 무관 변수 — 보존 확인용
        super::scrub_claude_session_env();
        assert!(std::env::var_os("CLAUDE_CODE_SESSION_ID").is_none());
        assert!(std::env::var_os("CLAUDE_CODE_CHILD_SESSION").is_none());
        assert!(std::env::var_os("CLAUDECODE").is_none());
        assert_eq!(
            std::env::var("CYS_SCRUB_TEST_KEEP").as_deref(),
            Ok("yes"),
            "무관 env까지 지우면 안 된다"
        );
        std::env::remove_var("CYS_SCRUB_TEST_KEEP");
    }
}

#[cfg(test)]
mod abi_severity_tests {
    use crate::severity::Severity;

    /// T1-6: AbiError → T1-3 Severity 사상이 §4.2 계약과 일치하는지 박제.
    /// Drift/LenMismatch=Critical(격리), VersionSkew=Recoverable(graceful).
    #[test]
    fn abi_error_to_severity() {
        assert_eq!(super::abi_severity(&cys::wire::AbiError::Drift), Severity::Critical);
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::LenMismatch),
            Severity::Critical
        );
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::VersionSkew {
                peer_pv: 2,
                local_pv: cys::wire::PROTO_PV
            }),
            Severity::Recoverable
        );
        // 격리 술어와의 정합: Critical만 격리, Recoverable은 재시도.
        assert!(super::abi_severity(&cys::wire::AbiError::Drift).is_critical());
        assert!(!super::abi_severity(&cys::wire::AbiError::VersionSkew {
            peer_pv: 2,
            local_pv: cys::wire::PROTO_PV
        })
        .is_critical());
    }
}

#[cfg(test)]
mod attach_race_tests {
    use crate::state::Daemon;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// ★회귀 박제 (state.rs reader thread ↔ main.rs run_attach 불변식):
    /// run_attach는 parser 락 아래에서 `out_tx.subscribe()`+화면 스냅샷을 원자적으로 뜬다
    /// (main.rs:538-542). 그 불변식이 성립하려면 reader 스레드도 `parser.process(chunk)`와
    /// `out_tx.send(chunk)`를 같은 parser 락 임계영역에 묶어야 한다. 둘이 분리되면
    /// (과거 버그) 다음 인터리빙이 같은 청크를 스냅샷·live 양쪽에 중복 배달한다:
    ///   ① reader: process(C) 후 락 해제
    ///   ② attach: 락 획득→subscribe(rx)→스냅샷(C 반영됨)→락 해제
    ///   ③ reader: out_tx.send(C) → ②의 rx가 C를 live로 수신  ⇒ C가 스냅샷+live 중복
    ///
    /// 이 테스트는 run_attach가 하는 일(락 아래 subscribe+스냅샷)을 그대로 모사하는 관측자를
    /// 실제 Surface reader 스레드와 동시에 돌려, "스냅샷 시점에 파서에 이미 반영된 마지막
    /// 청크가 그 직후 새 rx로 live 도착하는" 중복 창이 닫혔는지 다회 검증한다. 버그(분리)면
    /// 충분한 반복에서 중복이 잡히고, 수정(결합)이면 불변식이 무조건 성립해 0건이다.
    ///
    /// 핵심 신호: parser 락을 쥔 채 화면에 반영된 출력 바이트 수(=process가 본 누적 바이트)와
    /// 같은 락 구간에서 subscribe한 rx로 이후 도착하는 바이트가 겹치면(겹친 청크 존재) 중복.
    /// 마커를 청크 단위로 유일하게 만들어 "스냅샷에 보였는데 live로도 온" 마커를 직접 센다.
    #[test]
    fn process_and_send_are_atomic_under_parser_lock_no_dup_delivery() {
        // 멀티스레드 런타임 불필요 — 동기 스레드만 사용. PTY reader는 create_surface가
        // 내부에서 std::thread로 띄운다.
        let tmp = std::env::temp_dir().join(format!(
            "cys-attach-race-{}-{}.sock",
            std::process::id(),
            now_nanos()
        ));
        let daemon = Daemon::new(tmp.clone());

        // 출력 스트림: 각 라인은 유일 토큰 "MK<seq>E". reader 스레드가 끊임없이 청크
        // 경계를 만들도록 긴 루프로 연속 출력하며, 32라인마다 짧은 양보(usleep 미사용 —
        // 셸 내장만)로 reader/observer가 process↔send 경계를 다수 통과하게 한다.
        const N: usize = 6000;
        let script = format!(
            "i=0; while [ $i -lt {N} ]; do printf 'MK%dE\\n' $i; i=$((i+1)); done; sleep 3"
        );
        let surface = daemon
            .create_surface(None, Some(script), None, None, 35, 120)
            .expect("create_surface");

        // 다수 관측자 스레드: run_attach의 '락-아래 subscribe+스냅샷'을 그대로 모사하며
        // process↔send 분리 시 열리는 중복 창(스냅샷에 이미 보인 마커가 새 rx로 live 도착)을
        // 동시 다발로 두드린다. 여러 스레드가 경합해야 좁은 창에 안정적으로 착지한다.
        const OBSERVERS: usize = 6;
        let mut handles = Vec::new();
        for _ in 0..OBSERVERS {
            let surf = Arc::clone(&surface);
            handles.push(std::thread::spawn(move || {
                let mut dup_incidents: Vec<usize> = Vec::new();
                loop {
                    if surf.exited.load(Ordering::Relaxed) {
                        break;
                    }
                    // ── run_attach와 동일: parser 락 아래 subscribe + 스냅샷 ──
                    let (mut rx, snapshot_markers) = {
                        let parser = surf.parser.lock().unwrap_or_else(|e| e.into_inner());
                        let rx = surf.out_tx.subscribe();
                        let snap = parser.screen().contents();
                        (rx, parse_markers(snap.as_bytes()))
                    };
                    // 스냅샷에 마지막으로 보인(=파서에 이미 반영된) 마커. 이 마커는
                    // 결합(수정) 시 '이미 send 완료'라 새 rx로는 절대 오면 안 된다.
                    let Some(&last_in_snapshot) = snapshot_markers.iter().max() else {
                        continue;
                    };
                    // 새 rx를 잠깐 비워 live 마커를 수집 (non-blocking try_recv 폴링).
                    let mut live: Vec<usize> = Vec::new();
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_micros(500);
                    while std::time::Instant::now() < deadline {
                        match rx.try_recv() {
                            Ok(bytes) => live.extend(parse_markers(&bytes)),
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                                std::thread::yield_now()
                            }
                            Err(_) => break,
                        }
                    }
                    // 중복 판정: 스냅샷에 보였던(≤last_in_snapshot) 마커가 live로도 도착하면
                    // 그 청크가 스냅샷·live 양쪽에 배달된 것 — run_attach 주석이 막겠다던 케이스.
                    // (수정본은 process↔send가 원자적이라 새 rx에는 항상 >last_in_snapshot만 온다.)
                    for m in &live {
                        if *m <= last_in_snapshot {
                            dup_incidents.push(*m);
                        }
                    }
                }
                dup_incidents
            }));
        }

        let mut dup_incidents: Vec<usize> = Vec::new();
        for h in handles {
            dup_incidents.extend(h.join().expect("observer thread"));
        }

        // 정리: surface 종료 유도 (자력 종료 전에 kill — 좀비 방지)
        if let Ok(mut child) = surface.child.lock() {
            let _ = child.kill();
        }
        let _ = std::fs::remove_file(&tmp);

        assert!(
            dup_incidents.is_empty(),
            "process↔send가 parser 락에서 분리되어 청크 중복 배달 발생: {} 건 (예: {:?}). \
             reader 스레드는 process(chunk)와 out_tx.send(chunk)를 같은 parser 락 \
             임계영역에 묶어야 한다.",
            dup_incidents.len(),
            &dup_incidents[..dup_incidents.len().min(8)]
        );
    }

    /// "MK<n>E" 토큰을 바이트 스트림에서 추출 (청크/스냅샷 공통 파서).
    fn parse_markers(bytes: &[u8]) -> Vec<usize> {
        let s = String::from_utf8_lossy(bytes);
        let mut out = Vec::new();
        let mut rest = s.as_ref();
        while let Some(p) = rest.find("MK") {
            rest = &rest[p + 2..];
            if let Some(e) = rest.find('E') {
                if let Ok(n) = rest[..e].parse::<usize>() {
                    out.push(n);
                }
                rest = &rest[e + 1..];
            } else {
                break;
            }
        }
        out
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod feed_wait_disconnect_tests {
    use super::{handle_connection, Stream};
    use crate::state::Daemon;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    /// ★회귀 박제 (FeedWait 대기 중 클라이언트 끊김 + pause 동결):
    /// feed.push --wait의 대기 루프(main.rs)는 ① oneshot rx(=feed.reply) ② 1초 sleep ③ read
    /// half(끊김 감지) 세 가지를 select! 한다. ③이 없으면 워커가 응답 전에 연결을 끊어도
    /// 연결 태스크와 feed_waiters 엔트리가 timeout(최대 3600초)까지 살아남고, 데몬이 pause되면
    /// remaining이 영영 감소하지 않아(if !paused) timeout 분기에 절대 도달하지 못해 resume까지
    /// 무기한 잔존한다. 끊긴 워커가 pause 전후로 반복되면 연결 태스크·oneshot 채널이 단조 누적.
    ///
    /// 이 테스트는 ① feed.push --wait를 보내 waiter를 등록시키고 ② 데몬을 pause한 뒤
    /// ③ 클라이언트를 끊어, 연결 태스크가 (a) 유한 시간 내 종료하고 (b) feed_waiters 엔트리를
    /// 정리하는지 검증한다. 버그(③ 부재)면 pause 동결로 태스크가 영영 살아 timeout이 터지고
    /// waiter도 남는다. 수정(③ 존재)이면 끊김을 감지해 즉시 정리·종료한다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feed_wait_releases_waiter_when_client_disconnects_during_pause() {
        // ★상태 격리: state_dir = socket의 부모 디렉터리이고 거기에 feed.jsonl이 영속된다.
        // 소켓을 고유 하위 디렉터리에 두지 않으면 temp_dir/feed.jsonl을 다른 실행과 공유해
        // 직전 실행이 남긴 같은 request_id가 replay되어 'duplicate request_id'로 오염된다.
        let dir = std::env::temp_dir().join(format!(
            "cys-feedwait-disc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("cysd.sock");
        let daemon = Daemon::new(tmp.clone());

        // 인메모리 양방향 스트림: server는 handle_connection이, client는 테스트가 보유.
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection(Arc::clone(&daemon), server, None));

        // feed.push --wait — timeout_secs는 길게 줘서 끊김이 아닌 timeout으로 빠지는 오판을 배제.
        let mut client = client;
        let req = serde_json::json!({
            "id": "1",
            "method": "feed.push",
            "params": {
                "request_id": "disc-test-1",
                "kind": "approval",
                "title": "t",
                "body": "b",
                "wait": true,
                "timeout_secs": 3600
            }
        });
        let mut line = serde_json::to_vec(&req).unwrap();
        line.push(b'\n');
        client.write_all(&line).await.unwrap();
        client.flush().await.unwrap();

        // waiter 등록 대기 (FeedWait 진입 확인).
        let registered = wait_until(Duration::from_secs(5), || {
            daemon.feed_waiters.lock().unwrap().contains_key("disc-test-1")
        })
        .await;
        assert!(registered, "feed.push --wait가 waiter를 등록하지 못함");

        // 데몬 pause — 이 상태에서 timeout 카운트다운은 동결된다.
        daemon.paused.store(true, Ordering::Relaxed);

        // 클라이언트 끊김 (워커 프로세스 kill 모사).
        drop(client);

        // 수정본: 끊김을 감지해 유한 시간 내 연결 태스크 종료 + waiter 정리.
        // 버그: pause 동결로 영영 살아 timeout이 터진다.
        let finished = tokio::time::timeout(Duration::from_secs(10), conn).await;
        assert!(
            finished.is_ok(),
            "FeedWait 대기 태스크가 클라이언트 끊김을 감지하지 못해 종료하지 않음 \
             (pause 중 remaining 동결 → timeout 분기 영구 미도달)"
        );

        let waiter_cleared = daemon
            .feed_waiters
            .lock()
            .unwrap()
            .get("disc-test-1")
            .is_none();
        assert!(
            waiter_cleared,
            "끊김 후 feed_waiters['disc-test-1'] 엔트리가 정리되지 않고 잔존"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn wait_until<F: FnMut() -> bool>(limit: Duration, mut cond: F) -> bool {
        let deadline = std::time::Instant::now() + limit;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }
}

#[cfg(test)]
mod pipe_security_tests {
    use super::PIPE_SDDL_OWNER_ONLY;

    /// ★회귀 박제 (Windows named pipe = UDS 0o600 대칭 봉인):
    /// 기본 ServerOptions::create()는 lpSecurityAttributes=NULL로 파이프를 만들어
    /// 기본 DACL(같은 머신 임의 로컬 사용자에게 read/write 허용)을 받는다 — 인증 없는
    /// 제어 채널(send_text·send_key·ledger.kill)이 권한 우회로 노출되는 비대칭.
    /// 수정본은 owner-only SDDL을 SECURITY_ATTRIBUTES로 실어 creator·SYSTEM·Administrators만
    /// 접근하게 봉인한다. 이 테스트는 그 SDDL이 (a)광역 SID를 포함하지 않고 (b)보호된 DACL이며
    /// (c)owner를 명시 허용함을 단정해, 누군가 광역 권한을 다시 끼워넣거나 D:P를 떼어내면 깨진다.
    /// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로, SDDL 문자열 정합성으로 의도를 박제한다.)
    #[test]
    fn pipe_sddl_excludes_world_and_is_protected_owner_only() {
        let sddl = PIPE_SDDL_OWNER_ONLY;
        // (b) 보호된 DACL — 부모 ACL 상속을 차단해 광역 ACE가 흘러들지 않게 한다.
        assert!(
            sddl.starts_with("D:P"),
            "DACL must be protected (D:P) to block inherited world ACEs: {sddl}"
        );
        // (c) owner(creator)·SYSTEM·Administrators full-access ACE 존재.
        assert!(
            sddl.contains("(A;;FA;;;OW)"),
            "owner (OW) must have full access: {sddl}"
        );
        assert!(
            sddl.contains("(A;;FA;;;SY)") && sddl.contains("(A;;FA;;;BA)"),
            "SYSTEM (SY) and Administrators (BA) must be present: {sddl}"
        );
        // (a) 광역 SID 금지: Everyone(WD)·Authenticated Users(AU)·Anonymous(AN)·
        //     Network(NU)가 ACE로 들어오면 같은 머신/네트워크의 타 사용자가 접근 가능 → 회귀.
        for world in [";;;WD)", ";;;AU)", ";;;AN)", ";;;NU)"] {
            assert!(
                !sddl.contains(world),
                "broad SID {world} would re-open the pipe to other users: {sddl}"
            );
        }
        // deny ACE("D;")가 아닌 allow ACE("A;")만으로 구성 — 의도된 화이트리스트.
        assert!(
            !sddl.contains("(D;"),
            "owner-only seal should be an allow-list, not contain deny ACEs: {sddl}"
        );
    }

    /// ★회귀 박제 (Windows accept_loop의 connect() 오류 후 100% CPU spin 방지):
    /// 과거 Windows arm은 `loop { if server.connect().await.is_ok() { ... } }` 형태로
    /// 오류 분기가 전무했다. mio `ConnectNamedPipe`는 진짜 OS 오류를 즉시 Err로 돌려주고
    /// (정상 대기만 WouldBlock→tokio await) connecting 플래그도 즉시 해제하므로, 같은 broken
    /// 인스턴스에 sleep 없이 곧장 재시도하면 같은 Err가 무한 반복돼 tokio 워커 스레드가 영구
    /// 100% CPU를 태운다(자원 거버넌스를 표방하는 24/365 데몬에 치명적). 수정본은 오류 분기에서
    /// ①로그 ②인스턴스 재생성 ③backoff sleep로 회생한다. 그 backoff가 0이면 spin이 되살아나므로,
    /// 정책 상수가 non-zero임을 단정해 누가 다시 0/제거하면 깨지게 박제한다.
    /// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로 정책 상수 정합성으로 의도를 박제한다 —
    ///  PIPE_SDDL_OWNER_ONLY 박제와 같은 방식.)
    #[test]
    fn pipe_accept_error_backoff_is_nonzero_to_prevent_cpu_spin() {
        let backoff = super::PIPE_ACCEPT_ERROR_BACKOFF;
        assert!(
            !backoff.is_zero(),
            "accept-error backoff must be non-zero, else connect() Err re-tries on the same \
             broken pipe instance with no yield → 100% CPU spin: {backoff:?}"
        );
    }

    /// ★회귀 박제 (Windows named pipe 리스너 풀 — ERROR_PIPE_BUSY 231 봉인):
    /// named pipe 엔 UDS listen backlog 가 없어 '여분 listening 인스턴스 수'가 곧 동시 접속
    /// 수용량이다. 풀이 1로 돌아가면 accept→인스턴스 재생성 사이 창에 도착한 동시 접속
    /// (멀티 노드 RPC + GUI 기동 fan-out)이 전부 os error 231("모든 파이프 인스턴스가 사용 중")
    /// 로 튕긴다 — 2026-07-10 Windows GUI "startup failed" 실사고의 서버측 근원. 누가 풀을
    /// 다시 1로 줄이면 이 테스트가 깨진다. (Windows arm 은 이 호스트에서 컴파일/실행 불가하므로
    /// 정책 상수 정합성으로 의도를 박제한다 — PIPE_ACCEPT_ERROR_BACKOFF 박제와 같은 방식.)
    #[test]
    fn pipe_listener_pool_absorbs_concurrent_connects() {
        let pool = super::PIPE_LISTENER_POOL;
        assert!(
            pool >= 2,
            "listener pool must be ≥2, else concurrent connects hit ERROR_PIPE_BUSY(231) \
             in the accept→recreate window: {pool}"
        );
    }
}

#[cfg(test)]
mod auto_restore_tests {
    use super::{
        autorestore_retry_delay, decide_auto_restore, guard_restore_panic, loop_auto_restore_with,
        run_auto_restore_once, AutoRestore,
    };

    /// ★P0-7 회귀 잠금(D1/W5·CI 실경로 ⑧): 양 플랫폼 accept_loop 가 콜드부트 부트 공통 함수를 호출하는지 소스
    /// 수준으로 잠근다 — Windows accept_loop 에 배선이 빠져 auto-restore 가 발동조차 안 하던 P0-7 최종 결함 재발
    /// 차단. 호출 형태가 정확히 2회(unix+windows)여야 한다. (needle 은 concat! 으로 쪼개 이 테스트 자신을 세지
    /// 않게 한다 — 소스에 contiguous 리터럴이 없다.)
    #[test]
    fn post_listen_boot_wired_in_both_accept_loops() {
        let src = include_str!("main.rs");
        let needle = concat!("post_listen_boot", "(socket_path, &daemon)");
        let calls = src.matches(needle).count();
        assert_eq!(
            calls, 2,
            "콜드부트 부트 호출이 양 accept_loop(unix+windows)에 정확히 2회여야 한다(현재 {calls}회) — \
             한쪽 미배선/중복은 콜드부트 auto-restore 플랫폼 비대칭(P0-7) 재발"
        );
    }

    /// ★P0-5(D3/W5·CI 28780215417): auto-restore 스레드 panic 을 삼키지 않고 포착·기록하는지 — 재현 테스트.
    /// panic 하는 body → guard 는 false 반환(전파 안 함)·phoenix-restore.log 에 PANIC 기록. 정상 body → true.
    #[test]
    fn guard_restore_panic_catches_and_logs_no_propagation() {
        let dir = std::env::temp_dir().join(format!("cys-ar-panic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("phoenix-restore.log");

        // ① panic body → 포착(프로세스 안 죽음)·false 반환·로그에 PANIC 기록.
        let ok = guard_restore_panic(&log, || panic!("boom time.rs"));
        assert!(!ok, "panic 은 false 로 포착돼야 한다(전파 금지)");
        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            logged.contains("AUTO-RESTORE THREAD PANIC") && logged.contains("boom time.rs"),
            "panic 이 phoenix-restore.log 에 기록돼야 한다(침묵사 금지): {logged}"
        );

        // ② 정상 body → true·body 실행됨.
        let ran = std::sync::atomic::AtomicBool::new(false);
        let ok2 = guard_restore_panic(&log, || {
            ran.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        assert!(ok2 && ran.load(std::sync::atomic::Ordering::SeqCst), "정상 body 는 true·실행");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// opt-out(CYS_NO_AUTORESTORE=1)이면 phoenix가 있어도 스폰하지 않는다.
    #[test]
    fn opted_out_never_spawns() {
        let dir = std::env::temp_dir().join(format!("cys-ar-optout-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        assert_eq!(
            decide_auto_restore(&dir, true, &bin, "/usr/bin:/bin", "sock:test"),
            AutoRestore::OptedOut
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★B1: 디스크 phoenix 부재여도 Ready(임베드 추출이 권위) — 과거 PhoenixMissing skip 은 폐기.
    /// args[0]=디스크 phoenix 경로(폴백 후보)로 유지된다.
    #[test]
    fn missing_disk_phoenix_still_ready_embed_authoritative() {
        let dir = std::env::temp_dir().join(format!("cys-ar-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        match decide_auto_restore(&dir, false, &dir, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { args, .. } => {
                assert!(args[0].ends_with("bin/javis_phoenix.py"), "폴백 후보 경로: {}", args[0]);
                // args = [phoenix, "--socket", <sock>, "restore", "--auto"] — W6/E1 소켓 명시 전달.
                assert_eq!(args[1], "--socket");
                assert_eq!(&args[3..], &["restore".to_string(), "--auto".to_string()]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// phoenix 설치 시 `python3 <phoenix> restore --auto` 스폰 스펙을 낸다(--auto 필수).
    #[test]
    fn present_phoenix_builds_auto_restore_command() {
        let dir = std::env::temp_dir().join(format!("cys-ar-ready-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let ph = bin.join("javis_phoenix.py");
        std::fs::write(&ph, "#!/usr/bin/env python3\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, args, .. } => {
                assert_eq!(program, "python3");
                assert_eq!(args[0], ph.to_string_lossy());
                // args = [phoenix, "--socket", <sock>, "restore", "--auto"] — W6/E1 소켓 명시 전달.
                assert_eq!(args[1], "--socket");
                assert_eq!(&args[3..], &["restore".to_string(), "--auto".to_string()]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★D4(W5): 인터프리터 해석 — 동봉 runtime python3 이 실존하면 program 은 **그 절대경로**여야 한다
    /// ("python3" 리터럴 폴백이 아니라). 순정 Windows(python3 부재)·mac CLT 미설치 소비자에서 첫 스폰 단절
    /// (P0-7·P1-9)을 절대경로로 끊는 핵심. 기존 present_phoenix_builds_auto_restore_command 는 **번들 부재
    /// 폴백**만 검증(program=="python3")했다 — 그 리터럴 단언만으로는 절대경로 해석 결함을 통과시킨다(설계 D4 지적).
    #[test]
    fn ready_prefers_bundled_python_absolute_path() {
        let dir = std::env::temp_dir().join(format!("cys-ar-bundlepy-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir(bin) 기준 동봉 runtime python 디렉터리에 python3 실행파일을 둔다(runtime_bin_dirs SOT 와 일치).
        //   mac: <exe_dir>/runtime/python/bin/python3 · win: <exe_dir>/runtime/python/python3.exe
        let (py_dir, py_name) = if cfg!(windows) {
            (bin.join("runtime").join("python"), "python3.exe")
        } else {
            (bin.join("runtime").join("python").join("bin"), "python3")
        };
        std::fs::create_dir_all(&py_dir).unwrap();
        let py_path = py_dir.join(py_name);
        std::fs::write(&py_path, "#!/bin/sh\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, .. } => {
                assert_eq!(
                    program,
                    py_path.to_string_lossy(),
                    "동봉 python3 실존 시 program 은 절대경로여야 한다(리터럴 'python3' 아님)"
                );
                assert_ne!(program, "python3", "리터럴 폴백이면 D4 결함(절대경로 미해석)");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1/B3: exe 옆에 cys 가 실존하면 PHOENIX_CYS 를 그 절대경로로 주입하고, exe_dir 가 PATH 에 없으면
    /// PATH 를 선두주입한다(GUI/데몬 최소 PATH 침묵사 근원 수리). "python3" 문자열 단언만으로는 불충분(D4).
    #[test]
    fn ready_injects_phoenix_cys_and_path_env() {
        let dir = std::env::temp_dir().join(format!("cys-ar-env-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir 에 실행가능 cys 스텁을 둔다(PHOENIX_CYS 주입 조건 = 파일 실존).
        let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
        let cys_path = bin.join(cys_name);
        std::fs::write(&cys_path, "#!/bin/sh\n").unwrap();
        // GUI/데몬 최소 PATH 모사 — exe_dir 미포함이라 선두주입이 일어나야 한다.
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin:/usr/sbin:/sbin", "sock:test") {
            AutoRestore::Ready { env, .. } => {
                let cys_env = env
                    .iter()
                    .find(|(k, _)| k == "PHOENIX_CYS")
                    .map(|(_, v)| v.clone());
                assert_eq!(
                    cys_env.as_deref(),
                    Some(cys_path.to_string_lossy().as_ref()),
                    "PHOENIX_CYS 는 exe 옆 cys 절대경로여야 한다"
                );
                let path_env = env
                    .iter()
                    .find(|(k, _)| k == "PATH")
                    .map(|(_, v)| v.clone())
                    .expect("PATH 선두주입이 있어야 한다(exe_dir 미포함 PATH)");
                assert!(
                    path_env.starts_with(bin.to_string_lossy().as_ref()),
                    "PATH 는 exe_dir 선두여야 한다: {path_env}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1/B3: exe 옆에 cys 가 없으면 PHOENIX_CYS 를 주입하지 않는다(존재하지 않는 경로 강제 주입으로
    /// 재차 FileNotFoundError 를 만들지 않는다 — phoenix 의 which→표준경로 폴백에 위임).
    #[test]
    fn ready_omits_phoenix_cys_when_absent() {
        let dir = std::env::temp_dir().join(format!("cys-ar-nocys-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { env, .. } => {
                assert!(
                    !env.iter().any(|(k, _)| k == "PHOENIX_CYS"),
                    "cys 부재 시 PHOENIX_CYS 무주입이어야 한다: {env:?}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::{
        bundled_python3, disk_fallback_verify, extract_phoenix_embed, phoenix_embed_files,
        phoenix_self_test,
    };
    use std::cell::RefCell;
    use std::time::Duration;

    /// ★(T-0147-7 W2 부수) `CYS_PHOENIX_EXTRACT_FAIL` 은 **프로세스 전역** env 이고, 이 시임을 쓰는
    /// 테스트와 정상 추출을 검증하는 테스트가 같은 테스트 바이너리에서 **병렬로** 돈다. 종전 코드는
    /// `set_var` → 본문 → `remove_var` 였는데 그 사이 창에서 형제 테스트가 시임을 관측해
    /// `b1_extract_writes_phoenix_and_deps` 가 "injected mid-extraction failure" 로 죽었다
    /// (실측 재현 — 선재 red). 게다가 본문 assert 가 실패하면 언와인딩이 remove_var 를 건너뛰어
    /// **누출이 영구화**된다. 수리는 두 겹이다:
    ///   ① 이 락으로 시임 set/remove 윈도를 **직렬화**한다 → `--test-threads` 값과 무관하게 통과.
    ///   ② 해제를 RAII(`cys::pack::EnvGuard`)로 바꿔 패닉 언와인딩에도 복원된다.
    /// (pack.rs 의 `PACK_ENV_LOCK` 이 ENV_PACK_DIR 에 대해 쓰는 것과 같은 패턴 — 선례 재사용.)
    static EXTRACT_FAIL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ★codex W4 fix1: 추출 중간 실패(seam CYS_PHOENIX_EXTRACT_FAIL) 시 partial root 즉시 정리 — phoenix-embed 잔여 0.
    #[test]
    fn b1_extract_mid_failure_leaves_no_partial_root() {
        let _serial = EXTRACT_FAIL_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let sd = std::env::temp_dir().join(format!("cys-b1mf-{}", std::process::id()));
        std::fs::create_dir_all(&sd).unwrap();
        let res = {
            let _g = cys::pack::EnvGuard::set("CYS_PHOENIX_EXTRACT_FAIL", "1");
            extract_phoenix_embed(&sd)
        }; // ← 여기서 EnvGuard drop = 시임 복원(패닉 경로 포함)
        assert!(res.is_err(), "주입된 중간 실패가 Err 여야 한다");
        // phoenix-embed 하위 child dir 0(즉시 정리 — 다음 부팅 prune 의존 금지).
        let root = sd.join("phoenix-embed");
        let children = std::fs::read_dir(&root).map(|r| r.count()).unwrap_or(0);
        assert_eq!(children, 0, "중간 실패 후 partial root 잔존");
        let _ = std::fs::remove_dir_all(&sd);
    }

    /// ★codex W4 fix2: 디스크 폴백은 script-only 가 아니라 phoenix closure 전체 대조.
    /// phoenix.py 는 일치해도 형제(javis_state_snapshot.py) 부재/stale 이면 거부(어느 rel 인지 보고).
    #[test]
    fn b1_disk_fallback_full_tree_verify() {
        let pack = std::env::temp_dir().join(format!("cys-b1ft-{}", std::process::id()));
        let bin = pack.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // 전 closure 를 임베드 내용 그대로 디스크에 배치 → verified(Ok).
        for (rel, content) in phoenix_embed_files() {
            let p = pack.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        let disk_phoenix = bin.join("javis_phoenix.py");
        assert!(disk_fallback_verify(&disk_phoenix).is_ok(), "전 closure 일치 → verified");
        // 형제 stale: snapshot.py 를 변조 → 거부(rel 명시).
        std::fs::write(bin.join("javis_state_snapshot.py"), "STALE-SNAPSHOT-DRIFT").unwrap();
        let e = disk_fallback_verify(&disk_phoenix).unwrap_err();
        assert!(e.contains("javis_state_snapshot.py"), "stale 형제 rel 미보고: {e}");
        // 형제 부재: snapshot.py 삭제 → 거부(부재 명시).
        std::fs::remove_file(bin.join("javis_state_snapshot.py")).unwrap();
        let e2 = disk_fallback_verify(&disk_phoenix).unwrap_err();
        assert!(e2.contains("javis_state_snapshot.py") && e2.contains("부재"), "부재 형제 미보고: {e2}");
        let _ = std::fs::remove_dir_all(&pack);
    }

    /// ★B1①: 임베드 추출이 phoenix.py + 형제 의존(javis_state_snapshot.py)을 버전+uuid 격리 디렉터리에
    /// 임베드 내용 그대로 쓴다. temp 누수 0: 정리 후 디렉터리 소멸.
    #[test]
    fn b1_extract_writes_phoenix_and_deps() {
        // ★같은 시임(전역 env)을 만지는 형제 테스트와 직렬화 — 이 테스트는 시임이 **없는** 상태를
        //   전제하므로, 형제의 set/remove 윈도와 겹치면 결정론이 깨진다(선재 red 의 근저원인).
        //   추가로 상속된 누출(이전 실행 잔재·외부 env)도 이 스코프에서 명시 제거한다.
        let _serial = EXTRACT_FAIL_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _clean = cys::pack::EnvGuard::remove("CYS_PHOENIX_EXTRACT_FAIL");
        let sd = std::env::temp_dir().join(format!("cys-b1x-{}", std::process::id()));
        std::fs::create_dir_all(&sd).unwrap();
        let (root, script) = extract_phoenix_embed(&sd).expect("추출 성공");
        assert!(script.ends_with("bin/javis_phoenix.py"));
        assert!(script.is_file(), "phoenix.py 추출 안됨");
        let snap = root.join("bin").join("javis_state_snapshot.py");
        assert!(snap.is_file(), "형제 의존 javis_state_snapshot.py 미추출");
        // 내용 == 임베드
        let embed_phoenix = phoenix_embed_files()
            .into_iter()
            .find(|(rel, _)| *rel == "bin/javis_phoenix.py")
            .map(|(_, c)| c)
            .unwrap();
        assert_eq!(std::fs::read_to_string(&script).unwrap(), embed_phoenix);
        // 버전+uuid 격리 경로
        assert!(root.parent().unwrap().ends_with("phoenix-embed"));
        // temp 누수 0
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!root.exists());
        let _ = std::fs::remove_dir_all(&sd);
    }

    /// ★B1③: 추출된 실 phoenix 가 --selftest 를 통과한다(python3 가용 시). self-test 게이트 실증.
    #[test]
    fn b1_self_test_passes_on_real_embed() {
        // ★플레이키 봉인(W4 감사): 이 테스트도 extract_phoenix_embed 를 호출하므로 시임이
        //   **없는** 상태를 전제한다 — 형제(b1_extract_mid_failure...)의 CYS_PHOENIX_EXTRACT_FAIL
        //   set/remove 윈도와 병렬로 겹치면 "injected mid-extraction failure" 로 죽는다.
        //   b1_extract_writes_phoenix_and_deps 와 동형으로 락 직렬화 + 상속 누출 명시 제거.
        let _serial = EXTRACT_FAIL_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _clean = cys::pack::EnvGuard::remove("CYS_PHOENIX_EXTRACT_FAIL");
        let py = match std::process::Command::new("python3").arg("--version").output() {
            Ok(o) if o.status.success() => "python3".to_string(),
            _ => {
                eprintln!("python3 미가용 — self-test 게이트 skip");
                return;
            }
        };
        let sd = std::env::temp_dir().join(format!("cys-b1st-{}", std::process::id()));
        std::fs::create_dir_all(&sd).unwrap();
        let (root, script) = extract_phoenix_embed(&sd).expect("추출 성공");
        assert!(phoenix_self_test(&py, &script), "실 임베드 self-test 실패");
        // 존재하지 않는 스크립트는 self-test 실패(정직 강등 경로)
        assert!(!phoenix_self_test(&py, &root.join("bin").join("nope.py")));
        std::fs::remove_dir_all(&root).unwrap();
        let _ = std::fs::remove_dir_all(&sd);
    }

    /// ★B1 temp 누수 0: 크래시로 남은 이전 추출 디렉터리를 부트 시 prune 한다(정리 후 phoenix-embed 비움).
    #[test]
    fn b1_prune_stale_embed_dirs() {
        use super::prune_stale_phoenix_embed;
        let sd = std::env::temp_dir().join(format!("cys-b1p-{}", std::process::id()));
        let root = sd.join("phoenix-embed");
        // 이전 실행 잔재 2개 모사(크래시로 cleanup 못한 것).
        for u in ["0.12.20-111-222", "0.12.20-333-444"] {
            std::fs::create_dir_all(root.join(u).join("bin")).unwrap();
            std::fs::write(root.join(u).join("bin").join("x.py"), "stale").unwrap();
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        prune_stale_phoenix_embed(&sd);
        // prune 후 잔재 0(디렉터리 자체는 남아도 하위 비움).
        let remaining = std::fs::read_dir(&root).map(|r| r.count()).unwrap_or(0);
        assert_eq!(remaining, 0, "prune 후 잔여 추출 디렉터리 존재");
        // phoenix-embed 부재(부트 첫 회)에서도 panic 없이 무해.
        let empty = std::env::temp_dir().join(format!("cys-b1p-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        prune_stale_phoenix_embed(&empty); // no-op·무패닉
        let _ = std::fs::remove_dir_all(&sd);
        let _ = std::fs::remove_dir_all(&empty);
    }


    /// ★B3: 동봉 runtime python3 가 있으면 program 은 그 절대경로(리터럴 "python3" 아님). mac 레이아웃
    /// (runtime/python/bin/python3)으로 검증 — 순정 Windows/mac CLT 미설치 첫 스폰 단절 수리의 핵심.
    #[cfg(target_os = "macos")]
    #[test]
    fn b3_bundled_python_absolute_path_preferred() {
        let dir = std::env::temp_dir().join(format!("cys-b3-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir=bin. 동봉 python: bin/runtime/python/bin/python3.
        let pybin = bin.join("runtime").join("python").join("bin");
        std::fs::create_dir_all(&pybin).unwrap();
        let py = pybin.join("python3");
        std::fs::write(&py, "#!/bin/sh\n").unwrap();
        assert_eq!(bundled_python3(&bin).as_deref(), Some(py.to_string_lossy().as_ref()));
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, .. } => {
                assert_eq!(program, py.to_string_lossy(), "동봉 python3 절대경로여야 한다");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★B3: 동봉 runtime 이 없으면 program 은 "python3" 리터럴(PATH 폴백).
    #[test]
    fn b3_no_bundled_python_falls_back_to_literal() {
        let dir = std::env::temp_dir().join(format!("cys-b3-nolit-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        assert_eq!(bundled_python3(&bin), None);
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, .. } => assert_eq!(program, "python3"),
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1(codex major): 1차 비0 → (수동 복원으로 대상 라이브) → 2차 NOOP(=0)·정확히 2회 실행·중복 스폰 0.
    /// run_auto_restore_once 대신 스크립트 러너를 주입해 sleep 0 으로 결정론 검증(60s 실 sleep 회귀 회피).
    #[test]
    fn retry_first_nonzero_then_noop_runs_twice() {
        let calls = RefCell::new(Vec::<u32>::new());
        // 1차=exit 1(비0) → 2차=exit 0(수동 복원 후 재산정 NOOP). 스폰은 각 attempt 1회씩만(중복 0).
        let scripted = |attempt: u32| -> Option<i32> {
            calls.borrow_mut().push(attempt);
            if attempt == 0 { Some(1) } else { Some(0) }
        };
        let runs = loop_auto_restore_with(scripted, Duration::from_millis(0));
        assert_eq!(runs, 2, "1차 비0→2차 실행이어야 한다(정확히 2회)");
        assert_eq!(*calls.borrow(), vec![0, 1], "attempt 0,1 각 1회 — 중복 스폰 0");
    }

    /// ★재시도 소진: 2차도 비0이면 무한 재시도 금지(정확히 2회에서 종료).
    #[test]
    fn retry_exhausts_after_one_retry() {
        let n = RefCell::new(0u32);
        let runs = loop_auto_restore_with(
            |_a| {
                *n.borrow_mut() += 1;
                Some(1)
            },
            Duration::from_millis(0),
        );
        assert_eq!(runs, 2, "비0 후 1회만 재시도 — 무한 루프 금지");
        assert_eq!(*n.borrow(), 2);
    }

    /// ★exit 5(BREAKER)·6(CORRUPT/identity)=재시도 금지 — 정확히 1회 실행.
    #[test]
    fn breaker_and_corrupt_never_retry() {
        for code in [5, 6] {
            let n = RefCell::new(0u32);
            let runs = loop_auto_restore_with(
                |_a| {
                    *n.borrow_mut() += 1;
                    Some(code)
                },
                Duration::from_millis(0),
            );
            assert_eq!(runs, 1, "exit {code} 는 재시도 금지(1회 실행)");
        }
    }

    /// ★성공(0)은 재시도 없음 — 1회 실행.
    #[test]
    fn success_runs_once() {
        let runs = loop_auto_restore_with(|_a| Some(0), Duration::from_millis(0));
        assert_eq!(runs, 1);
    }

    /// ★스폰 실패(None)도 비0 클래스 — 1회 재시도 후 소진(2회).
    #[test]
    fn spawn_failure_retries_once() {
        let runs = loop_auto_restore_with(|_a| None, Duration::from_millis(0));
        assert_eq!(runs, 2, "None(스폰 실패)도 1회 재시도 후 종료");
    }

    /// ★CYS_AUTORESTORE_RETRY_DELAY_MS override 파싱(기본 60000·override 반영).
    #[test]
    fn retry_delay_env_override() {
        // 기본값
        std::env::remove_var("CYS_AUTORESTORE_RETRY_DELAY_MS");
        assert_eq!(autorestore_retry_delay(), Duration::from_millis(60_000));
        // override — 이 테스트만 단일 스레드 실행 계약(--test-threads=1)이라 env 격리 안전.
        std::env::set_var("CYS_AUTORESTORE_RETRY_DELAY_MS", "0");
        assert_eq!(autorestore_retry_delay(), Duration::from_millis(0));
        std::env::remove_var("CYS_AUTORESTORE_RETRY_DELAY_MS");
    }

    /// ★T6-L1: RestoreRootGuard 수명 계약 — 정상 스코프·panic unwind·loop 다중 attempt 모든 경로에서
    /// restore_roots 가 정확히 비워진다(등록 해제의 유일 경로가 Drop 임을 고정). guard drop 이 빠지면
    /// 복원 종료 후 잔존 자손이 authoritative 면제를 얻는 A7 취약이 재발한다.
    #[test]
    fn restore_roots_cleared_on_all_paths_l1() {
        use crate::state::{Daemon, RestoreRootGuard};
        let dir = std::env::temp_dir().join(format!(
            "cys-l1-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));

        // ① 정상 스코프: 등록 중 1개, 스코프 종료(drop) 후 빔.
        {
            let _g = RestoreRootGuard::new(daemon.clone(), 4242, 111);
            assert_eq!(daemon.restore_roots.lock().unwrap().len(), 1, "등록 중 1개여야");
        }
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "정상 drop 후 restore_roots 가 비지 않았다"
        );

        // ② panic unwind: catch_unwind 안에서 guard 살아있는 채 panic → Drop 이 unwind 중 해제.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = RestoreRootGuard::new(daemon.clone(), 4243, 222);
            assert_eq!(daemon.restore_roots.lock().unwrap().len(), 1);
            panic!("unwind through guard");
        }));
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "panic unwind 후 restore_roots 가 비지 않았다 (A7/L1)"
        );

        // ③ loop 다중 attempt: 각 attempt 가 자기 guard 를 등록·해제 → attempt 중 정확히 1개, 종료 후 빔.
        let d2 = daemon.clone();
        let runs = loop_auto_restore_with(
            move |attempt| {
                let _g = RestoreRootGuard::new(d2.clone(), 5000 + attempt, 333);
                assert_eq!(
                    d2.restore_roots.lock().unwrap().len(),
                    1,
                    "attempt 중 정확히 1개여야(누적 0)"
                );
                if attempt == 0 { Some(1) } else { Some(0) } // 1차 비0 → 2차 실행
            },
            Duration::from_millis(0),
        );
        assert_eq!(runs, 2, "1차 비0→2차 실행(정확히 2회)");
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "loop 종료 후 restore_roots 가 비지 않았다 (L1)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★T6-L1(실 실행): run_auto_restore_once 가 자식을 spawn·reap(좀비 0)하고 종료 후 restore_roots 를
    /// 비우며 exit code 를 계약대로 매핑한다(status().code() 동형). sleep 자식으로 등록 창을 실재화한다.
    #[cfg(unix)]
    #[test]
    fn run_auto_restore_once_reaps_and_clears_l1() {
        use crate::state::Daemon;
        let dir = std::env::temp_dir().join(format!(
            "cys-l1run-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let log = dir.join("phoenix-restore.log");

        // sleep 후 특정 코드 종료 — 관측 창 확보 + exit 매핑 검증. wait() 가 reap 한다.
        let code = run_auto_restore_once(
            &daemon,
            "sh",
            &["-c".to_string(), "sleep 0.2; exit 7".to_string()],
            &[],
            &log,
        );
        assert_eq!(code, Some(7), "exit code 계약 매핑이 깨졌다(status().code() 동형)");
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "run_auto_restore_once 종료 후 guard drop 으로 restore_roots 가 비어야 한다 (L1)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod first_line_idle_tests {
    use super::{
        handle_connection_capped, parse_first_line_cap, Stream, FIRST_LINE_IDLE_MAX_SECS,
        FIRST_LINE_IDLE_SECS,
    };
    use crate::state::Daemon;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn temp_daemon(tag: &str) -> (std::path::PathBuf, Arc<Daemon>) {
        let dir = std::env::temp_dir().join(format!(
            "cys-u6-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let d = Daemon::new(dir.join("cysd.sock"));
        (dir, d)
    }

    /// 롤백 노브 진리표 — 순수 판정부만 시험한다(프로세스 env 무접촉).
    #[test]
    fn first_line_cap_rollback_knob() {
        assert_eq!(
            parse_first_line_cap(None),
            Some(Duration::from_secs(FIRST_LINE_IDLE_SECS))
        );
        assert_eq!(parse_first_line_cap(Some("0")), None, "0 = 상한 해제(개정 전 거동)");
        assert_eq!(parse_first_line_cap(Some(" 7 ")), Some(Duration::from_secs(7)));
        assert_eq!(
            parse_first_line_cap(Some("nope")),
            Some(Duration::from_secs(FIRST_LINE_IDLE_SECS)),
            "오타가 상한을 조용히 없애면 안 된다"
        );
        // ── ★거대값 축(2026-08-24) — 종전엔 이 축이 통째로 없었고, 그래서 뚫려 있었다 ──
        // 음수·비숫자는 이미 안전하다(parse 실패 → 기본값 · 바로 위 두 줄이 박제). 위험은
        // **거대 양수** 하나였다: 판정 결과가 그대로 `Instant::now() + cap` 이 되는데
        // std `Instant::add` 는 오버플로에서 **패닉**하므로, 그 값이 들어오면 **모든 연결
        // 핸들러 태스크가 패닉**하고 데몬은 accept 만 하는 전 RPC 불통 상태가 된다.
        assert_eq!(
            parse_first_line_cap(Some("-5")),
            Some(Duration::from_secs(FIRST_LINE_IDLE_SECS)),
            "음수는 종전대로 파싱 실패 → 기본값(이 축은 이미 안전했다)"
        );
        for giant in ["9223372036854775807", "18446744073709551615", "9300000000000000000"] {
            assert_eq!(
                parse_first_line_cap(Some(giant)),
                Some(Duration::from_secs(FIRST_LINE_IDLE_MAX_SECS)),
                "거대값 {giant} 이 클램프되지 않았다 — 연결 핸들러 전면 패닉 경로"
            );
        }
        // ★불변식 실측: 어떤 입력에서도 결과는 `Instant` 에 더할 수 있다(패닉 도달 불가).
        //   `checked_add` 가 `None` 이면 그것이 곧 프로덕션의 패닉 지점이다.
        for raw in [
            None,
            Some("0"),
            Some(" 7 "),
            Some("nope"),
            Some("-5"),
            Some("86400"),
            Some("86401"),
            Some("9223372036854775807"),
            Some("18446744073709551615"),
        ] {
            if let Some(cap) = parse_first_line_cap(raw) {
                assert!(
                    std::time::Instant::now().checked_add(cap).is_some(),
                    "raw={raw:?} → cap={cap:?} 이 Instant 오버플로를 만든다 \
                     (프로덕션의 `Instant::now() + cap` 이 패닉한다)"
                );
                assert!(
                    tokio::time::Instant::now().checked_add(cap).is_some(),
                    "raw={raw:?} → tokio Instant 오버플로(실제 사용 지점)"
                );
            }
        }
    }

    /// ★U-6(서버측) 회귀 박제: **연결만 잡고 한 줄도 보내지 않는 클라이언트**에서 연결 태스크가
    /// 유계 종료한다. 개정 전에는 `next_line_capped` 가 시간 무계라 이 태스크가 영구 잔존했다
    /// (태스크·핸들 누수. Windows 파이프 인스턴스 총 상한 255 에 대한 잔여 거리도 함께 갉는다 —
    /// 근거는 `FIRST_LINE_IDLE_SECS` 주석의 P1-4 정정문).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn silent_client_connection_is_reclaimed() {
        let (dir, daemon) = temp_daemon("silent");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection_capped(
            Arc::clone(&daemon),
            server,
            None,
            Some(Duration::from_millis(300)),
        ));
        // 클라이언트를 살려 둔 채(= EOF 를 주지 않는다) 아무것도 쓰지 않는다.
        let finished = tokio::time::timeout(Duration::from_secs(5), conn).await;
        assert!(
            finished.is_ok(),
            "첫 줄을 보내지 않는 연결이 회수되지 않았다(연결 태스크 영구 잔존)"
        );
        drop(client);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★계측 타당성(negation): **상한을 끄면(=개정 전 코드) 같은 연결이 회수되지 않는다.**
    /// 위 테스트의 종료가 '무언의 클라이언트라 어차피 끝나서'가 아니라 **상한이 일한 결과**임을
    /// 증명한다. 롤백 노브(`CYS_CONN_FIRST_LINE_SECS=0`)의 실효 확인이기도 하다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn silent_client_persists_without_the_cap_instrument_validity() {
        let (dir, daemon) = temp_daemon("silent-nocap");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection_capped(
            Arc::clone(&daemon),
            server,
            None,
            None, // 상한 해제 = 개정 전 거동
        ));
        let still_running = tokio::time::timeout(Duration::from_millis(700), conn).await;
        assert!(
            still_running.is_err(),
            "상한이 없는데도 연결이 스스로 회수됐다 — 위 회수 테스트가 상한을 시험하지 못한다"
        );
        drop(client); // EOF 로 태스크를 정리한다(테스트 누수 방지)
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★비회귀 박제(더 중요): **첫 줄을 보낸 뒤의 유휴는 끊지 않는다.**
    /// GUI(src-tauri `RPC_POOL`)는 소켓별 영속 연결을 재사용하고, 서버가 유휴 중 끊으면 다음
    /// RPC 가 `rpc_full` 의 AfterSend(재시도 금지) 분기로 떨어져 사용자에게 오류로 보인다.
    /// 상한을 '연결 유휴 전체'로 넓히면 이 테스트가 적색이 된다 — 판정 축의 경계선이다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn established_connection_survives_idle_beyond_the_cap() {
        let (dir, daemon) = temp_daemon("established");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let _conn = tokio::spawn(handle_connection_capped(
            Arc::clone(&daemon),
            server,
            None,
            Some(Duration::from_millis(200)),
        ));
        let mut client = BufReader::new(client);
        let ping = b"{\"id\":1,\"method\":\"system.ping\",\"params\":{}}\n";

        client.get_mut().write_all(ping).await.unwrap();
        client.get_mut().flush().await.unwrap();
        let mut first = String::new();
        tokio::time::timeout(Duration::from_secs(5), client.read_line(&mut first))
            .await
            .expect("첫 응답이 오지 않았다")
            .unwrap();
        assert!(first.contains("\"ok\":true"), "첫 응답이 성공이 아니다: {first}");

        // 상한의 5배를 유휴로 보낸다 — 확립된 연결은 살아 있어야 한다.
        tokio::time::sleep(Duration::from_millis(1000)).await;

        client.get_mut().write_all(ping).await.unwrap();
        client.get_mut().flush().await.unwrap();
        let mut second = String::new();
        let got = tokio::time::timeout(Duration::from_secs(5), client.read_line(&mut second))
            .await
            .expect("두 번째 응답이 오지 않았다(유휴 끊김 = GUI 영속 풀 파괴)")
            .unwrap();
        assert!(got > 0 && second.contains("\"ok\":true"), "유휴 뒤 왕복 실패: {second}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★P1-3 회귀 박제: **개행 1바이트로 첫 줄 상한을 무장 해제할 수 없다.**
    ///
    /// 【고친 결함】 `awaiting_first_line = false;` 가 `if line.is_empty() { continue; }` **앞**에
    /// 있었다. 그래서 클라이언트가 `"\n"` 하나만 보내고 침묵하면 그 빈 줄이 '첫 요청 줄'로
    /// 계상돼 상한이 통째로 풀렸고, 연결 태스크가 **영구 잔존**했다. 요청으로 파싱조차 되지
    /// 않는 줄이 상한을 해제한다는 것이 결함의 핵심이다.
    ///
    /// 【적색 증명】 수정 전 코드에서 이 검체는 `finished` 가 `Err`(=회수 안 됨)로 적색이다 —
    /// 위 `silent_client_persists_without_the_cap_instrument_validity` 가 "상한이 없으면 회수되지
    /// 않는다"를 이미 박제하므로, 빈 줄이 상한을 없앤다는 것이 곧 이 검체의 적색이다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blank_line_does_not_disarm_the_first_line_cap() {
        let (dir, daemon) = temp_daemon("blankline");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection_capped(
            Arc::clone(&daemon),
            server,
            None,
            Some(Duration::from_millis(300)),
        ));
        let mut client = client;
        // 개행 1바이트만 보내고 침묵한다(EOF 도 주지 않는다).
        client.write_all(b"\n").await.unwrap();
        client.flush().await.unwrap();
        let finished = tokio::time::timeout(Duration::from_secs(5), conn).await;
        assert!(
            finished.is_ok(),
            "빈 줄 하나로 첫 줄 상한이 풀려 연결이 영구 잔존했다(개행 1바이트 무장 해제)"
        );
        drop(client);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★P1-3 (두 번째 축): **빈 줄을 계속 흘려도** 상한을 밀어낼 수 없다.
    /// 상한이 반복마다 재장전되는 상대 상한이면 빈 줄 드립(상한보다 짧은 주기)으로 연결을
    /// 무한히 붙잡을 수 있다 — 그건 상한이 막으려던 wedge 그 자체다. 절대 데드라인
    /// (`timeout_at`)이 그 경로를 닫는다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blank_line_drip_cannot_push_the_first_line_deadline() {
        let (dir, daemon) = temp_daemon("blankdrip");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection_capped(
            Arc::clone(&daemon),
            server,
            None,
            Some(Duration::from_millis(400)),
        ));
        let mut client = client;
        // 상한(400ms)보다 짧은 주기로 빈 줄을 흘린다 — 상대 상한이면 영원히 살아남는다.
        // ★드립은 관측창(3초)보다 **오래** 돌아야 한다. 드립이 먼저 끝나면 그 뒤 한 번의 상한
        //   만료로 연결이 회수돼, 상대 상한에서도 이 검체가 초록이 된다(계측 무효).
        let drip = tokio::spawn(async move {
            for _ in 0..300 {
                // 30초분
                if client.write_all(b"\n").await.is_err() {
                    break; // 서버가 회수했다 — 여기서 끝난다
                }
                let _ = client.flush().await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            client // EOF 를 주지 않기 위해 소유권을 유지한 채 반환
        });
        let finished = tokio::time::timeout(Duration::from_secs(3), conn).await;
        assert!(
            finished.is_ok(),
            "빈 줄 드립이 첫 줄 데드라인을 무한히 밀어냈다(상대 상한 부활)"
        );
        drip.abort();
        let _ = drip.await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★비회귀: 빈 줄 뒤에 **진짜 요청**이 오면 종전대로 처리되고, 그 뒤 유휴는 끊지 않는다.
    /// (P1-3 이 빈 줄을 '계상하지 않는다'로 바꿨을 뿐 '거부한다'로 바꾸지 않았음을 박제한다.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blank_lines_before_a_real_request_are_still_tolerated() {
        let (dir, daemon) = temp_daemon("blankthenreq");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let _conn = tokio::spawn(handle_connection_capped(
            Arc::clone(&daemon),
            server,
            None,
            Some(Duration::from_millis(600)),
        ));
        let mut client = BufReader::new(client);
        let ping = b"{\"id\":1,\"method\":\"system.ping\",\"params\":{}}\n";
        client.get_mut().write_all(b"\n\n  \n").await.unwrap();
        client.get_mut().write_all(ping).await.unwrap();
        client.get_mut().flush().await.unwrap();
        let mut first = String::new();
        tokio::time::timeout(Duration::from_secs(5), client.read_line(&mut first))
            .await
            .expect("빈 줄 뒤 요청의 응답이 오지 않았다")
            .unwrap();
        assert!(first.contains("\"ok\":true"), "빈 줄 뒤 요청이 처리되지 않았다: {first}");

        // 상한의 5배 유휴 후에도 확립된 연결은 살아 있어야 한다(첫 줄 상한이 놓였다).
        tokio::time::sleep(Duration::from_millis(3000)).await;
        client.get_mut().write_all(ping).await.unwrap();
        client.get_mut().flush().await.unwrap();
        let mut second = String::new();
        let got = tokio::time::timeout(Duration::from_secs(5), client.read_line(&mut second))
            .await
            .expect("두 번째 응답이 오지 않았다 — 첫 줄 상한이 확립 연결까지 끊었다")
            .unwrap();
        assert!(got > 0 && second.contains("\"ok\":true"), "유휴 뒤 왕복 실패: {second}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 업데이트 잔해(.prev*) 회수 회귀 핀 — Windows 전용 경로의 **로직**을 다른 OS 에서 박제한다.
//
// 실기기(Windows) 재현이 불가능한 축은 "매핑된 PE 이미지의 삭제 거부" 하나뿐이라, 그 축만
// remove 클로저의 반환값으로 주입하고 나머지(판정 규칙·재귀·격리 대상 선정·깊이 상한·
// 격리함 내부 재격리 금지)는 실제 파일시스템으로 검증한다.
//   근거: T4-6 FAIL(run 32976498890 · 2026-08-26 · 잔존 9개) — 재설치 순간 살아있던 홀더가
//   runtime\*.exe|dll 을 매핑 중이라 remove_file 이 전건 실패했고, 종전 코드는 그 실패를
//   무음 스킵해 잔해가 runtime 트리에 그대로 남았다.
// ═══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod update_leftover_sweep_tests {
    use super::{
        bound_update_trash, guard_canonical_binaries, guard_log_lines, is_family_leftover,
        is_update_leftover, leftover_log_lines, plan_leftover_action, quarantine_file_name,
        run_update_leftover_maintenance, sweep_update_leftovers, GuardStats, LeftoverLog,
        LeftoverPlan, PrevCandidate, SweepStats, TrashBoundStats, MIN_PROMOTABLE_BYTES,
        TRASH_MAX_AGE_SECS, TRASH_MAX_BYTES, TRASH_MAX_ENTRIES, TRASH_MAX_RECLAIM_PER_BOOT,
        UPDATE_TRASH_DIR,
    };
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn workdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cys-prevsweep-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    /// 크기·mtime 을 지정해 파일을 만든다 — 유계 3축(개수·나이·바이트)을 결정론으로 재현하기 위함.
    /// `File::set_modified`(std · 1.75+)를 쓰므로 외부 크레이트·플랫폼 분기가 필요 없다.
    fn touch_at(p: &Path, mtime_secs: u64, size: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![b'x'; size]).unwrap();
        let f = std::fs::File::options().write(true).open(p).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_secs))
            .unwrap();
    }

    /// 테스트 기준시각 — 실기 시계와 무관하게 나이 축을 계산하기 위한 고정점(2026-08-26).
    const T0: u64 = 1_756_200_000;

    fn real_remove(p: &Path) -> bool {
        std::fs::remove_file(p).is_ok()
    }

    fn real_relocate(src: &Path, dest: &Path) -> bool {
        dest.parent()
            .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
            && std::fs::rename(src, dest).is_ok()
    }

    /// 가드 동결이 없는 종전 스윕 계약을 그대로 핀하기 위한 빈 hold(가드 도입 전과 동치).
    fn no_hold() -> HashSet<PathBuf> {
        HashSet::new()
    }

    /// 지정 크기·채움 바이트의 파일 — 가드 테스트에서 '온전한 재료'(≥64KiB)와 승격본의 정체
    /// (어느 파일이 정식이 됐는가)를 내용으로 구별하기 위함.
    fn touch_sized(p: &Path, size: u64, fill: u8) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![fill; size as usize]).unwrap();
    }

    /// ① 판정 규칙 — 설치기가 만드는 두 형식만 잔해다. 사용자 파일(`*.preview.*`)은 **불가침**.
    #[test]
    fn leftover_rule_matches_installer_forms_only() {
        // CYS_SWAP_IN_PLACE 3칸 체인 (nsis-hooks.nsh:130-141)
        for n in ["cysd.prev.exe", "cys.prev2.exe", "cysd.prev3.exe", "CYSD.PREV.EXE"] {
            assert!(is_update_leftover(n), "체인 잔해를 놓쳤다: {n}");
        }
        // unlock-sweep 의 <원본이름>.prev<rand> (nsis-hooks.nsh:355-371)
        for n in [
            "msys-2.0.dll.prev4213",
            "bash.exe.prev0",
            "python313.dll.prev99999",
            "vcruntime140.dll.prev",
        ] {
            assert!(is_update_leftover(n), "unlock-sweep 잔해를 놓쳤다: {n}");
        }
        // 과대매칭 금지 — 종전 `contains(".prev")` 는 아래를 전부 삭제했다.
        for n in [
            "notes.preview.png",
            "release.previous.json",
            "prev.exe",
            "cysd.prev.exe.bak",
            "cys.prevX.exe", // .prev 뒤가 숫자가 아니다 — 잔해 아님(W1-5 과매칭 회귀 핀)
            "cys.new.exe",   // 배치 직전 신본 — 문법 **밖**은 의도다(가드의 명시 규칙 전담)
            "readme.md",
        ] {
            assert!(!is_update_leftover(n), "살아있는 파일을 잔해로 오판했다: {n}");
        }
    }

    /// ② 격리본의 이름은 **다시 잔해로 판정**돼야 한다 — 다음 부트가 마저 지우는 유일한 기전.
    #[test]
    fn quarantined_name_is_itself_a_leftover() {
        for n in ["msys-2.0.dll.prev4213", "cysd.prev.exe", "bash.exe.prev0"] {
            let q = quarantine_file_name(n, 1_756_200_000, 7);
            assert!(
                is_update_leftover(&q),
                "격리본이 다음 스윕에서 회수 불가 이름이 됐다: {n} → {q}"
            );
        }
    }

    /// ③ 정상 경로 — 잔해는 삭제되고 살아있는 파일은 그대로. 디렉토리는 삭제 대상이 아니다.
    #[test]
    fn sweep_removes_leftovers_and_spares_live_files() {
        let root = workdir("plain");
        touch(&root.join("cysd.prev.exe"));
        touch(&root.join("cysd.exe"));
        touch(&root.join("runtime/git/usr/bin/bash.exe"));
        touch(&root.join("runtime/git/usr/bin/msys-2.0.dll.prev4213"));
        touch(&root.join("runtime/python/notes.preview.png"));
        std::fs::create_dir_all(root.join("runtime/x.prev12")).unwrap(); // 디렉토리는 무접촉

        let mut stats = SweepStats::default();
        sweep_update_leftovers(
            &root,
            12,
            &root.join(UPDATE_TRASH_DIR),
            1,
            &no_hold(),
            &mut real_remove,
            &mut real_relocate,
            &mut stats,
        );

        assert_eq!(
            stats,
            SweepStats { removed: 2, quarantined: 0, stuck: 0, stamp_failed: 0 },
            "계수 불일치"
        );
        assert!(!root.join("cysd.prev.exe").exists());
        assert!(!root.join("runtime/git/usr/bin/msys-2.0.dll.prev4213").exists());
        assert!(root.join("cysd.exe").exists(), "살아있는 바이너리를 지웠다");
        assert!(root.join("runtime/git/usr/bin/bash.exe").exists(), "살아있는 바이너리를 지웠다");
        assert!(root.join("runtime/python/notes.preview.png").exists(), "사용자 파일을 지웠다");
        assert!(root.join("runtime/x.prev12").is_dir(), "디렉토리를 건드렸다");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ④ ★T4-6 회귀의 본체 — 삭제가 전건 실패(=홀더 생존)해도 **runtime 트리는 비어야 한다**.
    ///    종전 구현은 여기서 무음 스킵했고, 그 결과가 `잔존 9개` FAIL 이었다.
    #[test]
    fn sweep_quarantines_when_delete_fails_so_runtime_tree_is_empty() {
        let root = workdir("locked");
        let trash = root.join(UPDATE_TRASH_DIR);
        touch(&root.join("cysd.prev.exe"));
        touch(&root.join("runtime/git/usr/bin/bash.exe.prev777"));
        touch(&root.join("runtime/python/python313.dll.prev12"));

        let mut stats = SweepStats::default();
        let mut never_removes = |_: &Path| false; // 매핑된 이미지 = 삭제 거부
        sweep_update_leftovers(
            &root,
            12,
            &trash,
            1_756_200_000,
            &no_hold(),
            &mut never_removes,
            &mut real_relocate,
            &mut stats,
        );

        assert_eq!(stats, SweepStats { removed: 0, quarantined: 3, stuck: 0, stamp_failed: 0 });
        // T4-6 이 세는 바로 그 값 = runtime\ 재귀 잔해 수. 격리 후에는 0 이어야 한다.
        let rt_left = walk(&root.join("runtime"))
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_update_leftover)
            })
            .count();
        assert_eq!(rt_left, 0, "runtime 트리에 잔해가 남았다(T4-6 FAIL 재현)");
        assert!(!root.join("cysd.prev.exe").exists(), "루트 체인 잔해가 남았다");
        assert_eq!(walk(&trash).len(), 3, "격리함에 3건이 모여야 한다");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑤ 스윕은 격리함 서브트리를 **통째로 건너뛴다** — 재격리(무한 이동)도, 삭제 시도도 없다.
    ///    소유자를 하나로 못박는 핀: 격리함 안의 회수·유계·등급은 `bound_update_trash` 전담이고,
    ///    그래야 `SweepStats::stuck` 이 "살아있는 트리의 봉쇄 실패" 하나만 뜻하게 된다.
    #[test]
    fn sweep_never_touches_trash_subtree() {
        let root = workdir("trash");
        let trash = root.join(UPDATE_TRASH_DIR);
        let held = trash.join("msys-2.0.dll.prev4213.prev1756200000000");
        touch(&held);
        touch(&trash.join("nested/deep.prev7")); // 격리함 하위 디렉토리도 재귀 금지

        let mut stats = SweepStats::default();
        let mut remove_must_not_run = |p: &Path| panic!("격리함 안의 파일을 삭제하려 했다: {p:?}");
        let mut relocate_must_not_run = |_: &Path, _: &Path| {
            panic!("격리함 안의 파일을 다시 격리하려 했다");
        };
        sweep_update_leftovers(
            &root,
            12,
            &trash,
            1,
            &no_hold(),
            &mut remove_must_not_run,
            &mut relocate_must_not_run,
            &mut stats,
        );
        assert_eq!(stats, SweepStats::default(), "격리함이 스윕 계수에 섞였다");
        assert!(held.exists(), "격리함 파일이 사라졌다");
        assert!(trash.join("nested/deep.prev7").exists(), "격리함 하위로 재귀했다");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑥ 깊이 상한 — 상한 밖 서브트리는 이번 부트에서 손대지 않는다(종전 계약 보존).
    #[test]
    fn sweep_respects_depth_cap() {
        let root = workdir("depth");
        touch(&root.join("a/b/deep.prev1"));

        let mut stats = SweepStats::default();
        sweep_update_leftovers(
            &root,
            2, // root(1) → a(2) 까지만 — b 는 못 본다
            &root.join(UPDATE_TRASH_DIR),
            1,
            &no_hold(),
            &mut real_remove,
            &mut real_relocate,
            &mut stats,
        );
        assert_eq!(stats, SweepStats::default(), "깊이 상한을 넘어 순회했다");
        assert!(root.join("a/b/deep.prev1").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── 격리함 유계·경고 등급 핀 ────────────────────────────────────────────────
    // 마감 ①(경고 등급)·②(유계)의 계약을 박제한다. 등급 판정의 정의처는 `over_bound()` 하나이므로
    // 임계 3축은 순수 함수로 진리표를 박고, 파일시스템 축(회수·정렬·예산·무접촉)은 실제 디렉토리로 건다.

    /// ⑦ **등급의 정의처** — 격리함에 남은 것은 평시엔 경고가 아니다(조치 불요 = ⚠ 금지).
    ///    승격은 세 축(개수·바이트·나이) 중 하나가 깨졌을 때만. 상한 **경계값**까지 조용해야 한다.
    #[test]
    fn trash_bound_predicate_escalates_only_past_a_threshold() {
        let quiet = |remaining: usize, bytes: u64, aged: usize| TrashBoundStats {
            reclaimed: 0,
            remaining,
            remaining_bytes: bytes,
            aged_stuck: aged,
            deferred: 0,
        };
        // 평시: T4-6 이 실제로 본 9개 대기 — 봉쇄 성공 후의 설계된 상태다.
        assert!(!quiet(9, 9 * 4 * 1024 * 1024, 0).over_bound(), "정상 대기를 경고로 올렸다");
        // 경계: 상한 '도달'은 아직 조용, '초과'부터 시끄럽다(off-by-one 고정).
        assert!(!quiet(TRASH_MAX_ENTRIES, 0, 0).over_bound(), "개수 상한 경계에서 조기 발화했다");
        assert!(quiet(TRASH_MAX_ENTRIES + 1, 0, 0).over_bound(), "개수 상한 초과를 삼켰다");
        assert!(!quiet(1, TRASH_MAX_BYTES, 0).over_bound(), "바이트 상한 경계에서 조기 발화했다");
        assert!(quiet(1, TRASH_MAX_BYTES + 1, 0).over_bound(), "바이트 상한 초과를 삼켰다");
        // 나이 축은 개수·바이트가 아무리 작아도 단독으로 승격시킨다(홀더 문제가 아닌 다른 원인 신호).
        assert!(quiet(1, 1, 1).over_bound(), "나이 초과 미회수를 삼켰다");
        assert!(!TrashBoundStats::default().over_bound(), "빈 격리함이 경고를 냈다");
    }

    /// ⑧ 평시 경로(파일시스템) — 홀더 생존으로 전건 회수 실패해도, 상한 아래면 승격하지 않는다.
    ///    T5 가 실제로 낸 `stuck=9` 가 바로 이 상태였다(경고가 아니라 대기).
    #[test]
    fn trash_pending_under_bound_is_not_a_warning() {
        let trash = workdir("bound-quiet");
        for i in 0..9 {
            touch_at(&trash.join(format!("msys-2.0.dll.prev{i}")), T0 - 60, 1024);
        }
        let mut stats = TrashBoundStats::default();
        let mut never_removes = |_: &Path| false;
        bound_update_trash(&trash, T0, &mut never_removes, &mut stats);

        assert_eq!(stats.remaining, 9, "잔존 계수가 실제 점유와 다르다");
        assert_eq!(stats.remaining_bytes, 9 * 1024, "바이트 계수가 실제 점유와 다르다");
        assert_eq!(stats.aged_stuck, 0);
        assert!(!stats.over_bound(), "평시 대기에 ⚠ 를 달았다(오탐 배너 재발)");
        let _ = std::fs::remove_dir_all(&trash);
    }

    /// ⑨ 나이 축 — 상한을 넘긴 미회수분은 단독으로 승격한다. 경계 바로 아래는 조용.
    #[test]
    fn trash_aged_stuck_escalates_at_the_age_threshold() {
        let trash = workdir("bound-age");
        let old = T0 - TRASH_MAX_AGE_SECS - 1;
        touch_at(&trash.join("python313.dll.prev1"), old, 8);
        touch_at(&trash.join("bash.exe.prev2"), T0 - TRASH_MAX_AGE_SECS, 8); // 경계 = 아직 아님

        let mut stats = TrashBoundStats::default();
        let mut never_removes = |_: &Path| false;
        bound_update_trash(&trash, T0, &mut never_removes, &mut stats);

        assert_eq!(stats.remaining, 2);
        assert_eq!(stats.aged_stuck, 1, "나이 경계 판정이 어긋났다(초과분만 1건이어야 한다)");
        assert!(stats.over_bound(), "나이 초과 미회수가 조용히 넘어갔다");
        let _ = std::fs::remove_dir_all(&trash);
    }

    /// ⑩ 회수 — 홀더가 죽은 것만 지워지고, 남은 것은 계수·바이트에 **그대로** 남는다.
    ///    (삭제 실패분을 계산에서 빼면 무한 성장이 은폐된다 — 이 장치가 막는 실패.)
    #[test]
    fn trash_reclaims_dead_holders_and_still_counts_the_survivors() {
        let trash = workdir("bound-reclaim");
        touch_at(&trash.join("a.dll.prev1"), T0 - 10, 100); // 홀더 사망 → 회수
        touch_at(&trash.join("b.dll.prev2"), T0 - 10, 200); // 홀더 생존 → 잔존
        touch_at(&trash.join("c.dll.prev3"), T0 - 10, 400); // 홀더 사망 → 회수

        let mut stats = TrashBoundStats::default();
        let mut selective = |p: &Path| {
            let held = p.file_name().and_then(|n| n.to_str()) == Some("b.dll.prev2");
            !held && std::fs::remove_file(p).is_ok()
        };
        bound_update_trash(&trash, T0, &mut selective, &mut stats);

        assert_eq!(stats.reclaimed, 2);
        assert_eq!(stats.remaining, 1);
        assert_eq!(stats.remaining_bytes, 200, "잔존 바이트가 생존자만 세지 않았다");
        assert!(!trash.join("a.dll.prev1").exists());
        assert!(trash.join("b.dll.prev2").exists(), "잠긴 격리본을 지운 척했다");
        assert!(!trash.join("c.dll.prev3").exists());
        let _ = std::fs::remove_dir_all(&trash);
    }

    /// ⑪ 정렬 — 처리는 **가장 오래된 것부터**다. 회수 예산이 모자랄 때 오래된 쪽이 먼저 비워지는
    ///    성질이 여기에 걸린다(예산이 없으면 정렬은 관측 불가한 내부 사정이 된다).
    #[test]
    fn trash_processes_oldest_first() {
        let trash = workdir("bound-order");
        // 생성 순서를 나이 순서와 **반대로** 둔다 — read_dir 순서에 기대는 구현이면 여기서 깨진다.
        touch_at(&trash.join("newest.dll.prev3"), T0 - 10, 8);
        touch_at(&trash.join("oldest.dll.prev1"), T0 - 3000, 8);
        touch_at(&trash.join("middle.dll.prev2"), T0 - 300, 8);

        let mut seen: Vec<String> = Vec::new();
        let mut record = |p: &Path| {
            seen.push(p.file_name().unwrap().to_string_lossy().into_owned());
            false
        };
        let mut stats = TrashBoundStats::default();
        bound_update_trash(&trash, T0, &mut record, &mut stats);

        assert_eq!(
            seen,
            vec!["oldest.dll.prev1", "middle.dll.prev2", "newest.dll.prev3"],
            "오래된 것부터 돌지 않았다"
        );
        let _ = std::fs::remove_dir_all(&trash);
    }

    /// ⑫ 예산 — 한 부트의 회수 시도는 상한이 있고, 그 예산은 **오래된 것부터** 쓴다.
    ///    ★예산 밖으로 밀린 항목도 `remaining`/`remaining_bytes` 에 **반드시** 포함된다
    ///    (안 세면 격리함이 조용히 무한히 자란다 — 은폐 금지).
    #[test]
    fn trash_reclaim_budget_defers_newest_but_never_hides_them() {
        let trash = workdir("bound-budget");
        let over = 5usize;
        let total = TRASH_MAX_RECLAIM_PER_BOOT + over;
        for i in 0..total {
            // i 가 클수록 새 파일. 이름은 정렬 tie-break 이 아니라 mtime 으로 갈리게 한다.
            touch_at(&trash.join(format!("img{i:05}.dll.prev1")), T0 - (total - i) as u64, 4);
        }
        let mut attempts = 0usize;
        let mut newest_touched = false;
        let mut record = |p: &Path| {
            attempts += 1;
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            if n == format!("img{:05}.dll.prev1", total - 1) {
                newest_touched = true;
            }
            false
        };
        let mut stats = TrashBoundStats::default();
        bound_update_trash(&trash, T0, &mut record, &mut stats);

        assert_eq!(attempts, TRASH_MAX_RECLAIM_PER_BOOT, "부트 예산을 넘겨 시도했다");
        assert!(!newest_touched, "예산을 새 파일부터 썼다(오래된 것부터가 아니다)");
        assert_eq!(stats.deferred, over);
        assert_eq!(stats.remaining, total, "예산 밖 항목이 잔존 계수에서 빠졌다(무한 성장 은폐)");
        assert_eq!(stats.remaining_bytes, (total * 4) as u64);
        assert!(stats.over_bound(), "상한을 한참 넘겼는데 조용했다");
        let _ = std::fs::remove_dir_all(&trash);
    }

    /// ⑬ ★음성 대조(오너 앵커 ④) — 유계 장치는 **격리함 안의, 우리가 붙인 이름**만 만진다.
    ///    남의 파일·디렉토리·심볼릭링크·격리함 밖은 어떤 경로로도 삭제되지 않는다.
    #[test]
    fn trash_bound_never_reaches_outside_its_own_names() {
        let root = workdir("bound-safety");
        let trash = root.join(UPDATE_TRASH_DIR);
        // 격리함 **안**: 우리 이름 1건(회수 대상) + 남의 파일 2건 + 디렉토리 1건
        touch_at(&trash.join("ours.dll.prev1"), T0 - 10, 8);
        touch_at(&trash.join("notes.preview.png"), T0 - 10, 8);
        touch_at(&trash.join("README.txt"), T0 - 10, 8);
        std::fs::create_dir_all(trash.join("subdir.prev9")).unwrap();
        // 격리함 **밖**: 잔해 이름이어도 bound 는 읽지도 않는다(스윕의 관할이다).
        touch_at(&root.join("outside.dll.prev1"), T0 - 10, 8);
        touch_at(&root.join("runtime/live.exe"), T0 - 10, 8);
        // 심볼릭링크: 링크 너머의 살아있는 파일로 삭제가 새는 경로 차단.
        #[cfg(unix)]
        {
            let victim = root.join("runtime/live.exe");
            std::os::unix::fs::symlink(&victim, trash.join("link.dll.prev2")).unwrap();
        }

        let mut stats = TrashBoundStats::default();
        let mut real = |p: &Path| std::fs::remove_file(p).is_ok();
        bound_update_trash(&trash, T0, &mut real, &mut stats);

        assert_eq!(stats.reclaimed, 1, "우리 격리본 1건만 회수돼야 한다");
        assert!(!trash.join("ours.dll.prev1").exists());
        for keep in [
            trash.join("notes.preview.png"),
            trash.join("README.txt"),
            root.join("outside.dll.prev1"),
            root.join("runtime/live.exe"),
        ] {
            assert!(keep.exists(), "만지면 안 되는 파일을 지웠다: {keep:?}");
        }
        assert!(trash.join("subdir.prev9").is_dir(), "격리함 안 디렉토리를 지웠다");
        #[cfg(unix)]
        {
            assert!(
                trash.join("link.dll.prev2").symlink_metadata().is_ok(),
                "심볼릭링크를 삭제 대상으로 삼았다"
            );
            assert!(root.join("runtime/live.exe").exists(), "링크 너머로 삭제가 샜다");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑬-b 격리함 **자신**이 심볼릭링크면 회수를 통째로 포기한다 — 삭제가 링크 너머 남의 트리로
    ///      새는 경로 차단. 못 지우고 남는 것은 다음 부트가 재시도하지만, 잘못 지운 것은 못 되돌린다.
    #[cfg(unix)]
    #[test]
    fn trash_that_is_a_symlink_is_refused_outright() {
        let root = workdir("bound-symlink-trash");
        let victim_dir = root.join("someone-elses");
        touch_at(&victim_dir.join("their.dll.prev1"), T0 - 10, 8);
        let trash = root.join(UPDATE_TRASH_DIR);
        std::os::unix::fs::symlink(&victim_dir, &trash).unwrap();

        let mut stats = TrashBoundStats::default();
        let mut must_not_run = |p: &Path| panic!("링크 너머 파일을 삭제하려 했다: {p:?}");
        bound_update_trash(&trash, T0, &mut must_not_run, &mut stats);

        assert_eq!(stats, TrashBoundStats::default(), "링크 격리함을 처리했다");
        assert!(victim_dir.join("their.dll.prev1").exists(), "링크 너머로 삭제가 샜다");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑭ 부트 파이프라인 전체(스윕 → 유계) — main() 의 호출 순서를 그대로 재현한 통합 핀.
    ///    홀더 전건 생존 시: 설치 트리 잔해 0(봉쇄 성공·`stuck=0`) + 격리함 대기 + **조용**.
    #[test]
    fn boot_pipeline_quarantines_then_bounds_and_stays_quiet_when_healthy() {
        let root = workdir("bound-pipeline");
        let trash = root.join(UPDATE_TRASH_DIR);
        touch(&root.join("cysd.prev.exe"));
        touch(&root.join("cysd.exe")); // 정식 존재 — W1 가드가 가족 잔해를 스윕에 되돌리는 상태
        touch(&root.join("runtime/git/usr/bin/msys-2.0.dll.prev4213"));
        touch(&root.join("runtime/python/notes.preview.png")); // 사용자 파일 — 불가침
        touch(&root.join("runtime/git/usr/bin/bash.exe")); // 살아있는 바이너리 — 불가침

        let mut sweep = SweepStats::default();
        let mut never_removes = |_: &Path| false; // 홀더 전건 생존
        sweep_update_leftovers(
            &root,
            12,
            &trash,
            T0,
            &no_hold(),
            &mut never_removes,
            &mut real_relocate,
            &mut sweep,
        );
        let mut bound = TrashBoundStats::default();
        bound_update_trash(&trash, T0, &mut never_removes, &mut bound);

        assert_eq!(
            sweep,
            SweepStats { removed: 0, quarantined: 2, stuck: 0, stamp_failed: 0 },
            "봉쇄가 깨졌다"
        );
        assert_eq!(bound.remaining, 2, "격리본이 유계 계수에 잡히지 않았다");
        assert!(!bound.over_bound(), "건강한 상태에서 ⚠ 가 울렸다");
        let live_left = walk(&root)
            .into_iter()
            .filter(|p| !p.starts_with(&trash))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_update_leftover)
            })
            .count();
        assert_eq!(live_left, 0, "살아있는 설치 트리에 잔해가 남았다");
        assert!(root.join("runtime/python/notes.preview.png").exists());
        assert!(root.join("runtime/git/usr/bin/bash.exe").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑮ ★마감 ① 의 본체 — **부팅마다 울리던 ⚠ 가 평시엔 사라진다.**
    ///    T5 가 낸 그 상태(`stuck=9`)를 새 구조로 옮겨 재현: 9건은 격리함 안에서 대기 중이고,
    ///    사용자가 할 수 있는 조치가 없으므로 loud 는 0줄이어야 한다. 장수 홀더를 가진 실사용자가
    ///    부팅마다 보던 경고가 바로 이 줄이었다(오탐 배너 클래스 — 반복되면 신호가치가 죽는다).
    #[test]
    fn healthy_pending_trash_emits_no_warning_only_one_info_line() {
        let trash = PathBuf::from("C:\\Program Files\\cys\\.cys-update-trash");
        let sweep = SweepStats { removed: 0, quarantined: 9, stuck: 0, stamp_failed: 0 };
        let bound = TrashBoundStats {
            reclaimed: 0,
            remaining: 9,
            remaining_bytes: 9 * 4 * 1024 * 1024,
            aged_stuck: 0,
            deferred: 0,
        };
        let lines = leftover_log_lines(&sweep, &bound, &trash);
        assert_eq!(lines.len(), 1, "평시에 줄이 늘었다: {lines:?}");
        assert_eq!(lines[0].0, LeftoverLog::Info, "정상 대기를 ⚠ 로 올렸다");
        assert!(!lines[0].1.contains('⚠'), "info 줄에 경고 기호가 섞였다");
        assert!(lines[0].1.contains("pending=9"), "점유가 로그에서 사라졌다: {}", lines[0].1);

        // 완전히 조용한 부트(할 일 없음)는 **한 줄도** 내지 않는다.
        assert!(
            leftover_log_lines(&SweepStats::default(), &TrashBoundStats::default(), &trash)
                .is_empty(),
            "아무 일도 없는 부트가 로그를 냈다"
        );

        // 체류시계를 못 심은 경우도 **info 로만** 남는다 — 그 항목은 나이 축에서 '신선'으로
        // 접히므로 사용자가 할 조치가 없다(경고는 조치 가능할 때만 울린다).
        let unstamped = SweepStats { removed: 0, quarantined: 2, stuck: 0, stamp_failed: 2 };
        let pending = TrashBoundStats {
            reclaimed: 0,
            remaining: 2,
            remaining_bytes: 8,
            aged_stuck: 0,
            deferred: 0,
        };
        let lines = leftover_log_lines(&unstamped, &pending, &trash);
        assert!(
            lines.iter().all(|(l, _)| *l == LeftoverLog::Info),
            "스탬프 실패를 ⚠ 로 올렸다: {lines:?}"
        );
        assert!(
            lines.iter().any(|(_, s)| s.contains("격리 시각 기록 실패 2건")),
            "스탬프 실패를 통째로 삼켰다: {lines:?}"
        );
    }

    /// ⑯ 승격 경로 — 봉쇄 실패(살아있는 트리 잔존)와 유계 이탈은 **각각** loud 를 낸다.
    #[test]
    fn broken_containment_and_broken_bound_each_go_loud() {
        let trash = PathBuf::from("/tmp/.cys-update-trash");
        // 봉쇄 실패: 삭제도 격리도 실패 → 잔해가 살아있는 설치 트리에 남았다.
        let lines = leftover_log_lines(
            &SweepStats { removed: 0, quarantined: 0, stuck: 3, stamp_failed: 0 },
            &TrashBoundStats::default(),
            &trash,
        );
        assert_eq!(lines.iter().filter(|(l, _)| *l == LeftoverLog::Loud).count(), 1);
        assert!(lines.iter().any(|(_, s)| s.contains("회수 불가")));

        // 유계 이탈: 격리는 됐지만 상한을 넘었다 → 대기로는 설명되지 않는다.
        let over = TrashBoundStats {
            reclaimed: 0,
            remaining: TRASH_MAX_ENTRIES + 1,
            remaining_bytes: 1024,
            aged_stuck: 2,
            deferred: 0,
        };
        let lines = leftover_log_lines(&SweepStats::default(), &over, &trash);
        let loud: Vec<_> = lines.iter().filter(|(l, _)| *l == LeftoverLog::Loud).collect();
        assert_eq!(loud.len(), 1, "유계 이탈 고지는 1회 loud 여야 한다: {lines:?}");
        assert!(loud[0].1.contains("유계 이탈"), "{}", loud[0].1);
        assert!(loud[0].1.contains(&TRASH_MAX_ENTRIES.to_string()), "상한값이 안 보인다");
    }

    /// ⑰ ★부트 진입점 전체를 실제 파일시스템으로 돌린다 — `main()` 의 `cfg(windows)` 블록이
    ///    호출하는 바로 그 함수다. 종전에는 이 경로 전체가 타 OS 에서 **컴파일조차 되지 않아**
    ///    어느 CI 레인에서도 타입체크되지 않았다(Windows CI 는 `--bin cysd` 를 돌리지 않는다).
    ///    2 부트 수렴까지 건다: 부트1 이 회수·정리하고, 부트2 는 할 일이 없어 **한 줄도 내지 않는다**.
    #[test]
    fn maintenance_entrypoint_runs_end_to_end_and_converges_to_silence() {
        let root = workdir("maintenance");
        let trash = root.join(UPDATE_TRASH_DIR);
        touch(&root.join("cysd.prev.exe"));
        touch(&root.join("runtime/git/usr/bin/msys-2.0.dll.prev4213"));
        touch(&root.join("runtime/python/notes.preview.png")); // 사용자 파일 — 불가침
        touch(&root.join("cysd.exe")); // 살아있는 바이너리 — 불가침
        touch_at(&trash.join("old.dll.prev1"), T0 - 10, 32); // 지난 부트의 격리본(홀더 사망)

        // ── 부트 1 ──
        let lines = run_update_leftover_maintenance(&root, T0);
        assert!(
            lines.iter().all(|(l, _)| *l == LeftoverLog::Info),
            "건강한 부트에서 ⚠ 가 울렸다: {lines:?}"
        );
        assert_eq!(lines.len(), 1, "info 는 1줄이어야 한다: {lines:?}");
        assert!(lines[0].1.contains("removed=2"), "{}", lines[0].1);
        assert!(lines[0].1.contains("reclaimed=1"), "{}", lines[0].1);
        assert!(lines[0].1.contains("pending=0"), "{}", lines[0].1);
        assert!(!trash.exists(), "빈 격리함이 흔적으로 남았다");
        assert!(root.join("cysd.exe").exists(), "살아있는 바이너리를 지웠다");
        assert!(root.join("runtime/python/notes.preview.png").exists(), "사용자 파일을 지웠다");

        // ── 부트 2: 수렴 — 할 일이 없으면 로그도 없다(부팅마다의 소음 0) ──
        assert!(
            run_update_leftover_maintenance(&root, T0 + 1).is_empty(),
            "수렴 후에도 부트가 로그를 냈다"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑱ ★나이 축 **입력**의 회귀 핀 — **sweep 이 방금 격리한 항목은 `aged_stuck` 이 아니다.**
    ///
    ///    격리는 `fs::rename` 이라 원본 mtime 이 보존되는데, 격리 대상의 대종인 runtime PE 이미지는
    ///    업스트림 아카이브(PortableGit·Python embeddable 등)에서 풀려 **수개월 전 mtime** 을 갖는다.
    ///    나이 축이 그 값을 읽으면 격리되는 순간 이미 14일 상한을 넘겨 있어 **업데이트 직후 첫
    ///    부팅부터** ⚠ 가 뜬다 — 이 수리가 없애려던 오탐 배너 클래스 그 자체다
    ///    (CI 실측: 격리함 체류는 ~7분인데 aged 8건).
    ///
    ///    기존 핀 ⑨(`trash_aged_stuck_escalates_at_the_age_threshold`)는 mtime 을 **합성 주입**해
    ///    술어만 보므로 이 사각(입력이 틀린 경우)을 못 잡는다 — 그래서 그 핀을 유지한 채 **추가**한다.
    #[test]
    fn freshly_quarantined_item_is_not_aged_even_when_its_content_is_ancient() {
        let root = workdir("stamp-residency");
        let trash = root.join(UPDATE_TRASH_DIR);
        let ancient = T0 - 200 * 24 * 60 * 60; // 업스트림 아카이브가 준 mtime(수개월 전)
        touch_at(&root.join("runtime/git/usr/bin/msys-2.0.dll.prev4213"), ancient, 64);
        touch_at(&root.join("cysd.prev.exe"), ancient, 64);

        let mut sweep = SweepStats::default();
        let mut never_removes = |_: &Path| false; // 홀더 생존 = 삭제 거부 → 격리로 완결
        sweep_update_leftovers(
            &root,
            12,
            &trash,
            T0,
            &no_hold(),
            &mut never_removes,
            &mut real_relocate,
            &mut sweep,
        );
        assert_eq!(sweep.quarantined, 2, "격리가 안 됐다 — 이 핀의 전제가 깨졌다");
        assert_eq!(sweep.stamp_failed, 0, "격리 시각을 심지 못했다(체류시계 미기록)");

        // 같은 부트의 유계 패스 — 체류시간 0초다.
        let mut bound = TrashBoundStats::default();
        bound_update_trash(&trash, T0, &mut never_removes, &mut bound);
        assert_eq!(bound.remaining, 2);
        assert_eq!(bound.aged_stuck, 0, "방금 격리한 항목을 나이초과로 판정했다(오탐 배너 재발)");
        assert!(!bound.over_bound(), "업데이트 직후 첫 부팅에 ⚠ 가 울렸다");

        // ★대조 — 나이 축을 **죽인 게 아니라 입력을 고친 것**임을 같은 핀에서 못박는다.
        //   체류가 실제로 상한을 넘기면 그때는 승격한다(⑨ 의 술어가 여전히 살아있다).
        let mut later = TrashBoundStats::default();
        bound_update_trash(
            &trash,
            T0 + TRASH_MAX_AGE_SECS + 1,
            &mut never_removes,
            &mut later,
        );
        assert_eq!(later.aged_stuck, 2, "체류가 상한을 넘겼는데 조용했다(나이 축이 죽었다)");
        assert!(later.over_bound(), "나이 초과 미회수가 승격되지 않았다");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⑲ ★실패 방향 규율 — 체류시계를 **쓸 수 없는** 항목은 나이 축에서 '신선'으로 접힌다.
    ///    스탬프 실패가 조기 경고를 유발하는 방향은 금지다(경고는 조치 가능할 때만 울린다).
    ///    늦게 우는 것은 안전하고, 일찍 우는 것은 오탐 배너 클래스의 재발이다.
    #[cfg(unix)]
    #[test]
    fn unstampable_item_folds_to_fresh_instead_of_warning_early() {
        use std::os::unix::fs::PermissionsExt;
        let trash = workdir("bound-unstampable");
        let p = trash.join("python313.dll.prev1");
        touch_at(&p, T0 - TRASH_MAX_AGE_SECS - 1, 8); // 남의 시각(=스탬프가 실패했을 때 남는 값)
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        if std::fs::File::options().write(true).open(&p).is_ok() {
            // root 로 도는 러너에서는 '쓸 수 없는 파일'을 만들 수 없다 — 이 축은 재현 불가.
            let _ = std::fs::remove_dir_all(&trash);
            return;
        }

        let mut stats = TrashBoundStats::default();
        let mut never_removes = |_: &Path| false;
        bound_update_trash(&trash, T0, &mut never_removes, &mut stats);

        assert_eq!(stats.remaining, 1, "잔존 계수에서 빠졌다(성장 은폐)");
        assert_eq!(stats.aged_stuck, 0, "스탬프를 못 쓰는 항목을 나이초과로 올렸다(조기 경고)");
        assert!(!stats.over_bound(), "조치할 수 없는 항목에 ⚠ 를 달았다");
        let _ = std::fs::remove_dir_all(&trash);
    }

    // ── W1 부팅 잔해 회수 가드 핀 — "정식 부재 시 잔해는 복구 재료다"(T3 사고 계열 차단) ──

    /// G① 스펙 W1-1 — 정식 부재 + prev 1개: **삭제 0건**, prev 가 rename 으로 정식에 승격된다.
    ///    종전 스윕은 여기서 유일한 동작본을 지웠다(CLI 영구 소실) — 이 가드의 존재 이유.
    #[test]
    fn guard_promotes_lone_prev_instead_of_deleting_it() {
        let root = workdir("guard-lone-prev");
        touch_sized(&root.join("cys.prev.exe"), MIN_PROMOTABLE_BYTES, b'P');

        let lines = run_update_leftover_maintenance(&root, T0);

        let canonical = root.join("cys.exe");
        assert!(canonical.exists(), "정식이 재건되지 않았다");
        assert_eq!(
            std::fs::metadata(&canonical).unwrap().len(),
            MIN_PROMOTABLE_BYTES,
            "승격본이 prev 의 내용이 아니다"
        );
        assert!(!root.join("cys.prev.exe").exists(), "승격은 rename 이어야 한다(원본 잔류)");
        assert!(!root.join(UPDATE_TRASH_DIR).exists(), "복구 재료가 격리함으로 갔다");
        assert_eq!(lines.len(), 1, "복구 1건 = loud 1줄이어야 한다: {lines:?}");
        assert!(
            lines[0].0 == LeftoverLog::Loud
                && lines[0].1.contains("부팅 복구")
                && lines[0].1.contains("cys.exe"),
            "복구가 유음으로 기록되지 않았다: {lines:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G② 스펙 W1-2 — 정식 부재 + `.new`(≥64KiB) + prev 2개: `.new` 가 1순위로 승격되고,
    ///    잔여 prev 는 **승격 성공 뒤** 종전 규칙(스윕)으로 정리된다.
    #[test]
    fn guard_prefers_new_binary_then_sweeps_the_prevs() {
        let root = workdir("guard-new-first");
        touch_sized(&root.join("cysd.new.exe"), MIN_PROMOTABLE_BYTES + 7, b'N');
        touch_at(&root.join("cysd.prev.exe"), T0 - 10, MIN_PROMOTABLE_BYTES as usize);
        touch_at(&root.join("cysd.prev2.exe"), T0 - 20, MIN_PROMOTABLE_BYTES as usize);

        let lines = run_update_leftover_maintenance(&root, T0);

        assert_eq!(
            std::fs::metadata(root.join("cysd.exe")).unwrap().len(),
            MIN_PROMOTABLE_BYTES + 7,
            "승격본이 .new 가 아니다(우선순위 위반)"
        );
        assert!(!root.join("cysd.new.exe").exists(), ".new 가 rename 되지 않고 남았다");
        assert!(
            !root.join("cysd.prev.exe").exists() && !root.join("cysd.prev2.exe").exists(),
            "승격 후 잔여 prev 가 정리되지 않았다"
        );
        assert!(
            lines
                .iter()
                .any(|(l, s)| *l == LeftoverLog::Loud && s.contains("부팅 복구")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|(_, s)| s.contains("removed=2")),
            "잔여 prev 정리가 계수되지 않았다: {lines:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G③ 스펙 W1-3 — 정식 존재 + prev 3개 + `.new`: 종전과 동일하게 정리되고, 문법 밖의
    ///    `.new` 는 "정식이 존재할 때만 삭제"라는 명시 규칙으로 함께 지워진다. ⚠ 는 없다.
    #[test]
    fn guard_with_canonical_present_sweeps_prevs_and_deletes_new() {
        let root = workdir("guard-canonical-ok");
        touch_sized(&root.join("cys.exe"), MIN_PROMOTABLE_BYTES, b'C');
        touch(&root.join("cys.prev.exe"));
        touch(&root.join("cys.prev2.exe"));
        touch(&root.join("cys.exe.prev4213")); // unlock-sweep 형식도 같은 가족
        touch(&root.join("cys.new.exe"));

        let lines = run_update_leftover_maintenance(&root, T0);

        assert!(root.join("cys.exe").exists(), "정식을 건드렸다");
        for gone in ["cys.prev.exe", "cys.prev2.exe", "cys.exe.prev4213", "cys.new.exe"] {
            assert!(!root.join(gone).exists(), "{gone} 이 정리되지 않았다");
        }
        assert!(
            lines.iter().all(|(l, _)| *l == LeftoverLog::Info),
            "정상 정리에 ⚠ 가 울렸다: {lines:?}"
        );
        assert!(lines.iter().any(|(_, s)| s.contains("removed=3")), "{lines:?}");
        assert!(
            lines.iter().any(|(_, s)| s.contains(".new 신본 1건 삭제")),
            ".new 처분이 침묵했다: {lines:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G④ 스펙 W1-4 — 정식 부재 + **승격 가능한** 재료 전무(전부 미달 크기): 무행동 + 경고.
    ///    잘린 재료를 승격하면 다음 부팅이 "정식 존재"로 오판해 남은 재료를 지운다 — 그 전이를
    ///    여기서 끊는다. 대조: 가족 파일이 **하나도 없는** 바이너리(개발 트리의 cys-app 등)는
    ///    지킬 것이 없으므로 조용하다(정식 부재 ≠ 무조건 경고 — 매 부팅 우는 배너 금지).
    #[test]
    fn guard_holds_everything_loudly_when_no_usable_material_exists() {
        let root = workdir("guard-hold");
        touch(&root.join("cys.prev.exe")); // 1바이트 = 승격 부적격
        touch_sized(&root.join("cys.new.exe"), 4, b'T'); // 잘린 신본 = 승격 부적격

        let lines = run_update_leftover_maintenance(&root, T0);

        assert!(root.join("cys.prev.exe").exists(), "동결해야 할 복구 재료를 지웠다");
        assert!(root.join("cys.new.exe").exists(), "정식 부재인데 .new 를 지웠다");
        assert!(!root.join("cys.exe").exists(), "미달 재료를 승격했다");
        assert!(!root.join(UPDATE_TRASH_DIR).exists(), "복구 재료가 격리함으로 갔다");
        let loud: Vec<_> = lines.iter().filter(|(l, _)| *l == LeftoverLog::Loud).collect();
        assert_eq!(loud.len(), 1, "무행동 동결은 정확히 1회 loud 여야 한다: {lines:?}");
        assert!(
            loud[0].1.contains("복구 재료 전무") && loud[0].1.contains("cys.exe"),
            "{}",
            loud[0].1
        );
        // 가족이 통째로 없는 cysd·cys-app 은 침묵해야 한다(개발 트리가 매 부팅 울면 안 된다).
        assert!(
            !lines.iter().any(|(_, s)| s.contains("cys-app") || s.contains("cysd.exe")),
            "{lines:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G⑤ 판정 순수함수 진리표 — 우선순위(정식 > `.new` ≥64KiB > 최신 prev)·문턱·동률 결정론.
    #[test]
    fn plan_leftover_action_priority_thresholds_and_determinism() {
        let pc = |name: &str, mtime: u64, size: u64| PrevCandidate {
            name: name.into(),
            mtime,
            size,
        };
        let big = MIN_PROMOTABLE_BYTES;
        // 정식 존재 → 스윕 위임. `.new` 는 존재할 때만(크기 무관) 삭제 지시 — 명시 규칙.
        assert_eq!(
            plan_leftover_action(true, None, &[]),
            LeftoverPlan::SweepAll { delete_new: false }
        );
        assert_eq!(
            plan_leftover_action(true, Some(3), &[pc("cys.prev.exe", 9, big)]),
            LeftoverPlan::SweepAll { delete_new: true }
        );
        // 정식 부재: 온전한 `.new` 가 prev 보다 우선한다(경계 64KiB 정확히 = 승격 가능).
        assert_eq!(
            plan_leftover_action(false, Some(big), &[pc("cys.prev.exe", 9, big)]),
            LeftoverPlan::PromoteNew
        );
        // `.new` 미달(잘림) → 최신 mtime 의 prev 폴백 + 미달 신본은 승격 성공 후 처분 지시.
        assert_eq!(
            plan_leftover_action(
                false,
                Some(big - 1),
                &[pc("cys.prev.exe", 5, big), pc("cys.prev2.exe", 9, big)]
            ),
            LeftoverPlan::PromotePrev { source: "cys.prev2.exe".into(), delete_new: true }
        );
        // prev 중 미달 크기는 최신이어도 건너뛴다 — 잘린 본 승격은 다음 부팅의 "정식 존재" 오판
        // → 남은 진짜 재료 삭제로 전이된다(가드가 막는 사고 그 자체).
        assert_eq!(
            plan_leftover_action(false, None, &[pc("cys.prev.exe", 99, 10), pc("cys.prev2.exe", 5, big)]),
            LeftoverPlan::PromotePrev { source: "cys.prev2.exe".into(), delete_new: false }
        );
        // mtime 동률은 이름으로 깬다 — read_dir 순서와 무관한 부트 간 결정론.
        assert_eq!(
            plan_leftover_action(false, None, &[pc("cys.prev.exe", 7, big), pc("cys.prev3.exe", 7, big)]),
            LeftoverPlan::PromotePrev { source: "cys.prev3.exe".into(), delete_new: false }
        );
        // 가족이 통째로 없으면 조용(NothingToDo) · 미달 재료뿐이면 동결(Hold) — 둘은 다른 상태다.
        assert_eq!(plan_leftover_action(false, None, &[]), LeftoverPlan::NothingToDo);
        assert_eq!(plan_leftover_action(false, Some(8), &[]), LeftoverPlan::Hold);
        assert_eq!(
            plan_leftover_action(false, None, &[pc("cys.prev.exe", 9, 8)]),
            LeftoverPlan::Hold
        );
    }

    /// G⑥ 가족 판정 — 설치기 형식(체인·unlock-sweep·W4-B tick)은 가족이고, 이웃 바이너리·
    ///    남의 파일·`.new` 는 아니다. 확장자는 순수 로직의 매개변수다(exe 하드코딩 금지).
    #[test]
    fn family_membership_matches_exactly_one_binary() {
        for n in [
            "cys.prev.exe",      // 체인 슬롯
            "cys.prev2.exe",
            "cys.prev1756.exe",  // W4-B tick 슬롯 형식
            "cys.exe.prev4213",  // unlock-sweep 형식
            "CYS.PREV.EXE",      // Windows 대소문자 무구분
            "cys.prev",
        ] {
            assert!(is_family_leftover(n, "cys", "exe"), "가족을 놓쳤다: {n}");
        }
        for n in [
            "cysd.prev.exe",         // 이웃 바이너리의 잔해 — 접두 오염 금지
            "cys-app.prev.exe",
            "msys-2.0.dll.prev4213", // runtime 이미지 — 가드 범위 밖
            "cys.new.exe",           // 문법 밖(잔해 아님) — 명시 규칙 전담
            "cys.prevX.exe",         // 숫자 아님 = 잔해 아님(W1-5 과매칭 핀)
            "notes.preview.png",
        ] {
            assert!(!is_family_leftover(n, "cys", "exe"), "가족 과매칭: {n}");
        }
        // 확장자 무관(순수) — unlock-sweep 형식은 exe 없는 세계에서도 같은 문법이다.
        assert!(is_family_leftover("cys.bin.prev7", "cys", "bin"));
        assert!(!is_family_leftover("cys.bin.prev7", "cys", "exe"));
    }

    /// G⑦ 승격 실패는 **동결로 강등**된다 — "정식 부재인데 지운다"로 전이하는 분기가 없음을
    ///    주입 실패(rename 거부 = 잠긴 대상·권한·설치기 경합)로 박제한다.
    #[test]
    fn guard_failed_promotion_freezes_the_family_and_deletes_nothing() {
        let root = workdir("guard-promote-fail");
        touch_sized(&root.join("cys-app.new.exe"), MIN_PROMOTABLE_BYTES, b'N');
        touch_sized(&root.join("cys-app.prev.exe"), MIN_PROMOTABLE_BYTES, b'P');
        touch_sized(&root.join("cys-app.prev2.exe"), MIN_PROMOTABLE_BYTES, b'Q');

        let mut guard = GuardStats::default();
        let mut real_rm = |p: &Path| std::fs::remove_file(p).is_ok();
        let mut refuse_promote = |_: &Path, _: &Path| false;
        let hold =
            guard_canonical_binaries(&root, "exe", &mut real_rm, &mut refuse_promote, &mut guard);

        assert_eq!(
            guard.promote_failed,
            vec![("cys-app.exe".to_string(), "cys-app.new.exe".to_string())]
        );
        assert!(guard.promoted.is_empty() && guard.deleted_new == 0, "{guard:?}");
        assert_eq!(
            hold,
            [root.join("cys-app.prev.exe"), root.join("cys-app.prev2.exe")]
                .into_iter()
                .collect::<HashSet<_>>(),
            "동결 집합이 가족 잔해 전체가 아니다"
        );
        // 동결이 실제로 스윕을 이긴다 — hold 를 그대로 넘긴 스윕은 무접촉·무계수여야 한다.
        let mut sweep = SweepStats::default();
        sweep_update_leftovers(
            &root,
            12,
            &root.join(UPDATE_TRASH_DIR),
            T0,
            &hold,
            &mut real_rm,
            &mut real_relocate,
            &mut sweep,
        );
        assert_eq!(sweep, SweepStats::default(), "동결된 재료를 스윕이 만졌다");
        for keep in ["cys-app.new.exe", "cys-app.prev.exe", "cys-app.prev2.exe"] {
            assert!(root.join(keep).exists(), "승격 실패에서 {keep} 를 잃었다");
        }
        let lines = guard_log_lines(&guard);
        assert!(
            lines
                .iter()
                .any(|(l, s)| *l == LeftoverLog::Loud && s.contains("승격 실패")),
            "실패가 침묵했다: {lines:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G⑧ prev 여러 개면 **최신 mtime** 이 승격된다(실제 파일시스템 mtime 배선까지) —
    ///    가장 마지막까지 정식 자리에 있었던 본이 최신이다. unlock-sweep 형식도 후보다.
    #[test]
    fn guard_promotes_the_newest_prev_by_mtime() {
        let root = workdir("guard-newest-prev");
        touch_at(&root.join("cysd.prev.exe"), T0 - 100, MIN_PROMOTABLE_BYTES as usize);
        touch_at(
            &root.join("cysd.exe.prev777"),
            T0 - 5,
            MIN_PROMOTABLE_BYTES as usize + 3,
        );

        let lines = run_update_leftover_maintenance(&root, T0);

        assert_eq!(
            std::fs::metadata(root.join("cysd.exe")).unwrap().len(),
            MIN_PROMOTABLE_BYTES + 3,
            "최신 mtime 의 prev 가 아니라 다른 본이 승격됐다"
        );
        assert!(!root.join("cysd.exe.prev777").exists(), "승격은 rename 이어야 한다");
        assert!(!root.join("cysd.prev.exe").exists(), "승격 후 잔여 prev 가 정리되지 않았다");
        assert!(
            lines
                .iter()
                .any(|(l, s)| *l == LeftoverLog::Loud && s.contains("cysd.exe.prev777")),
            "{lines:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
