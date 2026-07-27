//! C2/C2-b(v0.13.17) — 큐 역압 정책 계층.
//!
//! 이 모듈이 담는 것: ①큐 이벤트 이름 상수(D10) ②Agent-origin 소프트캡 판정(D3)
//! ③dead-letter 원장 기록(C3 사양·전 사유 공유) ④발신 통계(C2-b).
//!
//! **위협 모델 명문화**: 이건 혼잡 게이트이지 보안 게이트가 아니다. origin 판별은 커널 검증
//! 발신 surface에서 파생하므로 pane 밖 프로세스(launchd 등)는 System으로 분류돼 정책을
//! 벗어난다 — 막으려는 것은 "실수 폭주"이지 악의적 우회가 아니다(설계 D2).
//!
//! **원장은 파일, 이벤트는 관측**: 이벤트 버스는 링(유실 가능)이므로 정책 판단·원장은
//! 이벤트에 의존하지 않는다. 사실의 SOT는 dead-letters.jsonl이다(D10).

use crate::state::{state_dir, Daemon, QueueItem, QueueOrigin, Surface};
use serde_json::json;
use std::sync::Arc;

/// D10: 큐 이벤트 이름 상수 — 리터럴 산재(오타·불일치)를 차단한다.
/// 기존 5종 + 신설 3종. 큐 카테고리 한정 국소 집약이며 전역 리팩토링은 의도적 비확대다.
pub mod queue_events {
    pub const CATEGORY: &str = "queue";

    // 기존 5종
    pub const ENQUEUED: &str = "queue.enqueued";
    pub const DELIVERED: &str = "queue.delivered";
    pub const DROPPED: &str = "queue.dropped";
    pub const DEPTH_HIGH: &str = "queue.depth_high";
    pub const CLEAR_DENIED: &str = "queue.clear_denied";

    // 신설 4종 (C2·C3·C4·C5)
    pub const REJECTED: &str = "queue.rejected";
    pub const EXPIRED: &str = "queue.expired";
    pub const MERGED: &str = "queue.merged";
    /// ★E5 재수정: 잔여 수명 부족으로 **병합이 성립하지 않아** 구 항목을 superseded 로 보내고
    /// 갱신본을 신규 발신으로 강등했다. `queue.merged` 와 구분해 발행한다 — 원장에 superseded
    /// 줄만 남고 merged 이벤트가 없으면 관측자가 불일치로 읽는다.
    pub const MERGE_DEMOTED: &str = "queue.merge_demoted";
    /// C5 OOB 제어 레인 통지 — governance::oob_notify 가 유일 발행자.
    pub const OOB_NOTIFIED: &str = "queue.oob_notified";
}

/// 정책 env(`CYS_QUEUE_*`)는 프로세스 전역이라, 이를 만지는 **모든 모듈의 테스트**가 이
/// 하나의 락을 잡아 직렬화한다. 모듈별 사설 락을 두면 queue_policy·handlers·governance 가
/// 서로를 모른 채 동시에 set_var 해서 간헐 실패(flake)가 난다 — 락은 env 만큼 전역이어야 한다.
#[cfg(test)]
pub(crate) static QUEUE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 기존 하드캡 — **불변**. origin 무관하게 큐 전체 길이에 적용된다.
pub const QUEUE_HARD_CAP: usize = 100;

/// Agent-origin 소프트캡(기본 25 · 0=비활성). 큐 전체가 아니라 **대상 큐 안의 Agent-origin
/// 항목 수**를 센다 — 실사고 실측(정상 depth ≤ 5 · 사고 시 76)에서 "정상의 5배·사고의 1/3".
pub fn agent_softcap() -> usize {
    std::env::var("CYS_QUEUE_AGENT_SOFTCAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(25)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyMode {
    /// 관찰 전용 — 이벤트만 남기고 적재는 허용한다. **명시 선택(`log`)일 때만** 성립한다.
    Log,
    /// 기본 — 결정론 거부. 발신자에게 오류 + 데몬이 dead-letter에 전문 기록.
    Enforce,
}

/// ★B1(0.13.21): 기본값을 Log→Enforce 로 뒤집는다. 종전 기본이 Log 였던 탓에 소프트캡의
/// 거부 경로가 **실배포에서 도달 불능**이었다(env 를 enforce 로 거는 배선이 어디에도 없었다) —
/// 억제 계층 전체가 死코드로 남아 폭주를 못 막았다. 관찰 기간은 끝났고, 이제 차단이 기본이다.
///
/// 오타·미지값이 Enforce 로 떨어지는 것도 의도다: 억제 실패(폭주)가 오차단보다 해롭고,
/// 관찰 모드는 "그 모드를 쓰겠다"는 명시 선언이 있을 때만 켜져야 한다(fail-safe).
/// 구모드 복귀·롤백은 `CYS_QUEUE_POLICY_MODE=log`.
pub fn policy_mode() -> PolicyMode {
    match std::env::var("CYS_QUEUE_POLICY_MODE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "log" => PolicyMode::Log,
        _ => PolicyMode::Enforce,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyVerdict {
    Allow,
    /// 소프트캡 도달 — log 모드: 이벤트만 남기고 적재는 허용.
    LogOnly,
    /// 소프트캡 도달 — enforce 모드: 거부 + dead-letter.
    Reject,
}

/// 소프트캡 판정(순수 — 테스트 핀). System/Human origin은 어떤 모드에서도 정책 밖이다.
pub fn decide(
    origin_is_agent: bool,
    agent_depth: usize,
    softcap: usize,
    mode: PolicyMode,
) -> PolicyVerdict {
    if !origin_is_agent || softcap == 0 || agent_depth < softcap {
        return PolicyVerdict::Allow;
    }
    match mode {
        PolicyMode::Log => PolicyVerdict::LogOnly,
        PolicyMode::Enforce => PolicyVerdict::Reject,
    }
}

/// enforce 거부 시 발신자에게 주는 **결정론 후속 지시**. 재전송 폭풍을 문안으로 차단한다
/// (거부와 동시에 데몬이 전문을 dead-letter에 기록하므로 발신자가 다시 보낼 이유가 없다).
pub const SOFTCAP_DIRECTIVE: &str =
    "메시지는 dead-letter에 기록되었다. 재전송하지 마라. 전문을 파일로 남기고 작업을 계속하라.";
pub const ERR_SOFTCAP: &str = "queue_softcap_exceeded";

// ─────────────────────────────────────────────────────────────────────────────
// dead-letter 원장 (C3 사양 — TTL·거부·병합·operator clear가 공유)
// ─────────────────────────────────────────────────────────────────────────────

pub const DEAD_LETTER_FILE: &str = "dead-letters.jsonl";
/// 회전 임계 10MB — 무한 성장 차단. 회전본은 `dead-letters.<ts>.jsonl`.
pub const DEAD_LETTER_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// 회전본 보존 기간(일). 회전은 파일 하나의 무한 성장만 막았고 **회전본 개수**는 무한이었다 —
/// 24/365 데몬에서 state dir 이 단조 증가한다. 회전이 일어나는 순간(=드문 사건)에만 정리해
/// 상시 비용 0으로 유지한다. 현행 파일(`dead-letters.jsonl`)은 절대 prune 대상이 아니다.
pub const DEAD_LETTER_RETAIN_DAYS: u64 = 30;

/// 원장 사유. **어떤 경로로도 무음 삭제는 없다** — 큐에서 사라지는 모든 항목은 여기에 남는다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadLetterReason {
    Ttl,
    SoftcapRejected,
    Superseded,
    WakeupCap,
    /// 오퍼레이터 토큰으로 남의 큐를 비운 경우(위임형 clear).
    ClearedByOperator,
    /// 소유자 자신(또는 데몬 내부 익명 경로)이 자기 큐를 비운 경우.
    /// ★이 사유가 없던 동안 self/익명 clear 는 **무음 삭제**였다 — 불변식의 구멍이었다.
    Cleared,
    /// surface 가 오너 의도로 닫힘(close_surface).
    SurfaceClosed,
    /// pane 프로세스가 자력 종료(reader EOF).
    ProcessExited,
}

impl DeadLetterReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeadLetterReason::Ttl => "ttl",
            DeadLetterReason::SoftcapRejected => "softcap_rejected",
            DeadLetterReason::Superseded => "superseded",
            DeadLetterReason::WakeupCap => "wakeup_cap",
            DeadLetterReason::ClearedByOperator => "cleared_by_operator",
            DeadLetterReason::Cleared => "cleared",
            DeadLetterReason::SurfaceClosed => "surface_closed",
            DeadLetterReason::ProcessExited => "process_exited",
        }
    }
}

/// dead-letter 한 줄을 기록하고 파일 경로를 돌려준다.
///
/// 원자성: `O_APPEND` 단일 `write` 호출(한 줄 원자성) + 데몬 내 `Mutex<()>` 직렬화.
/// 핸들러 스레드(거부·superseded)와 watchdog(TTL)이 동시에 append 해도 라인이 섞이지 않는다.
///
/// ★기록 실패 의미론(fail-open toward no-data-loss): 실패하면 `Err`를 준다. **호출자는 이때
/// 항목을 제거하거나 거부를 강행해선 안 된다** — 원장 없는 삭제 금지가 무손실 원칙보다
/// 우선하는 규칙이 아니라, 무손실 원칙 그 자체다. 실패는 health 경고로 가시화한다.
pub fn record_dead_letter(
    daemon: &Daemon,
    surface_id: u64,
    role: Option<&str>,
    item: &QueueItem,
    reason: DeadLetterReason,
) -> Result<std::path::PathBuf, String> {
    let dir = state_dir(&daemon.socket_path);
    let path = dir.join(DEAD_LETTER_FILE);
    let line = json!({
        "ts": crate::state::now_epoch(),
        "surface_id": surface_id,
        "role": role,
        "text": item.text,
        "origin": item.origin,
        "reason": reason.as_str(),
        // 진단 보조(원장 판독용) — 계약 필드는 위 6종이다.
        "seq": item.seq,
        "kind": item.kind.as_str(),
        "enqueued_at": item.enqueued_at,
        "idem_key": item.idem_key,
        "merged_count": item.merged_count,
    });
    let mut buf = match serde_json::to_string(&line) {
        Ok(s) => s,
        Err(e) => return Err(format!("dead-letter serialize failed: {e}")),
    };
    buf.push('\n');

    let result = (|| -> std::io::Result<()> {
        let _g = daemon.dead_letter_lock.lock().unwrap();
        std::fs::create_dir_all(&dir)?;
        // 회전: 임계 도달 시 현재 파일을 타임스탬프 이름으로 옮긴다(락 보유 중 — 경합 없음).
        if let Ok(md) = std::fs::metadata(&path) {
            if md.len() >= DEAD_LETTER_MAX_BYTES {
                let rotated = dir.join(format!(
                    "dead-letters.{}.jsonl",
                    crate::state::now_epoch() as u64
                ));
                let _ = std::fs::rename(&path, rotated);
                // 회전 직후에만 보존기간 정리 — 상시 비용 0, 무한 누적 차단.
                prune_rotated_dead_letters(&dir, DEAD_LETTER_RETAIN_DAYS);
            }
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // 단일 write 호출 — 짧은 write로 쪼개지지 않게 write_all이 아니라 write를 쓰고
        // 부분 기록이면 오류로 올린다(반쪽 라인이 원장을 오염시키는 것보다 실패가 낫다).
        let n = std::io::Write::write(&mut f, buf.as_bytes())?;
        if n != buf.len() {
            return Err(std::io::Error::other("partial dead-letter write"));
        }
        Ok(())
    })();

    match result {
        Ok(()) => Ok(path),
        Err(e) => {
            let msg = format!("dead-letter write failed: {e}");
            daemon.bus.publish(
                "health.dead_letter_write_failed",
                "health",
                Some(surface_id),
                json!({"error": msg, "reason": reason.as_str(),
                       "hint": "원장을 못 남겼으므로 해당 항목은 제거·거부하지 않고 보류한다(무손실 우선)"}),
            );
            Err(msg)
        }
    }
}

/// 회전본(`dead-letters.<ts>.jsonl`) 중 mtime 이 `retain_days` 를 넘긴 것을 지운다.
/// **현행 파일은 이름이 다르므로 절대 걸리지 않는다**(패턴에 중간 세그먼트가 필수).
/// 순수 파일 연산이라 테스트가 mtime 조작으로 직접 핀할 수 있다.
pub fn prune_rotated_dead_letters(dir: &std::path::Path, retain_days: u64) -> usize {
    if retain_days == 0 {
        return 0;
    }
    let cutoff = match std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(retain_days * 86_400))
    {
        Some(t) => t,
        None => return 0,
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut pruned = 0usize;
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        // `dead-letters.<무언가>.jsonl` 만 — 현행 `dead-letters.jsonl` 은 중간 세그먼트가 없다.
        let Some(mid) = name
            .strip_prefix("dead-letters.")
            .and_then(|s| s.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if mid.is_empty() {
            continue;
        }
        let old = ent
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t < cutoff)
            .unwrap_or(false);
        if old && std::fs::remove_file(ent.path()).is_ok() {
            pruned += 1;
        }
    }
    pruned
}

/// 종료·clear 경로 공용 **record-before-remove**.
///
/// 종전 세 경로(queue.clear self·close_surface·reader EOF)는 `drain(..)` 으로 먼저 비우고
/// 나서 이벤트만 냈다 — 이벤트 버스는 유실 가능한 링이므로 사실상 **무음 삭제**였다.
/// 여기서는 순서를 뒤집는다: 항목별로 원장 기록을 먼저 시도하고, **기록에 성공한 것만**
/// 큐에서 제거한다. 실패분은 큐에 남아 다음 기회(재시도·WAL)로 보존된다.
///
/// 제거는 스냅샷 인덱스가 아니라 `seq` 집합으로 한다 — 기록 중 새 항목이 들어와도
/// 우리가 기록하지 않은 항목은 절대 지워지지 않는다(TOCTOU 안전).
pub struct DrainOutcome {
    /// 원장 기록 성공 = 실제로 큐에서 제거된 항목.
    pub cleared: Vec<QueueItem>,
    /// 원장 기록 실패 = 큐에 잔류시킨 항목(무손실 우선).
    pub retained: Vec<QueueItem>,
    pub role: Option<String>,
}

impl DrainOutcome {
    pub fn total(&self) -> usize {
        self.cleared.len() + self.retained.len()
    }
    pub fn cleared_bytes(&self) -> usize {
        self.cleared.iter().map(|i| i.bytes()).sum()
    }
}

pub fn record_then_drain(
    daemon: &Daemon,
    surface: &Surface,
    reason: DeadLetterReason,
) -> DrainOutcome {
    let items: Vec<QueueItem> = surface.pending_queue.lock().unwrap().iter().cloned().collect();
    let role = surface.role.lock().unwrap().clone();
    if items.is_empty() {
        return DrainOutcome { cleared: Vec::new(), retained: Vec::new(), role };
    }
    let mut cleared = Vec::new();
    let mut retained = Vec::new();
    for item in items {
        match record_dead_letter(daemon, surface.id, role.as_deref(), &item, reason) {
            Ok(_) => cleared.push(item),
            Err(_) => retained.push(item),
        }
    }
    if !cleared.is_empty() {
        let seqs: std::collections::HashSet<u64> = cleared.iter().map(|i| i.seq).collect();
        surface
            .pending_queue
            .lock()
            .unwrap()
            .retain(|i| !seqs.contains(&i.seq));
    }
    DrainOutcome { cleared, retained, role }
}

// ─────────────────────────────────────────────────────────────────────────────
// C2-b 발신 통계 — 기획서 §6.2 "규약 위반 기계 검출"의 L1 토대
// ─────────────────────────────────────────────────────────────────────────────

/// 발신 surface 단위 누적. 재시작 시 리셋 허용(관찰 창 내 상대 비교라 영속화는 비목표).
///
/// ★해석 한계(명문화): keyless 발신에는 정당한 [ACTION]·긴급 직접 send가 포함된다.
/// 이 수치는 **판정이 아니라 스크리닝 신호**다 — "keyless 급증 노드 = 점검 후보"이지
/// 자동 위반 판정 금지.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueSendStats {
    pub total: u64,
    pub keyless: u64,
    pub rejected: u64,
}

fn bump_stats(daemon: &Daemon, sender_sid: u64, keyless: bool, rejected: bool) {
    let mut m = daemon.queue_send_stats.lock().unwrap();
    let e = m.entry(sender_sid).or_default();
    e.total += 1;
    if keyless {
        e.keyless += 1;
    }
    if rejected {
        e.rejected += 1;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 공통 enqueue — enqueue 3개소(send_text · send_key · enqueue_master_wakeup)가 공유한다.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum EnqueueError {
    /// 기존 하드캡 100 — 발신자가 동기 오류로 즉시 안다(원장 기록 대상 아님).
    /// 단 데몬 내부 무음 경로(wakeup)는 호출자가 dead-letter로 승격한다(D9③).
    HardCap,
    /// Agent-origin 소프트캡 도달 + enforce 모드. dead-letter 경로는 이미 기록 시도했다.
    Softcap {
        /// 기록 성공 시 원장 경로. `None`이면 기록 실패 = 원장 없는 거부이므로
        /// 호출자는 거부를 강행하지 않고 **수용**한다(fail-open toward no-data-loss).
        dead_letter: Option<String>,
        agent_depth: usize,
        softcap: usize,
    },
}

/// 적재 결과 — 신규 적재인지 기존 항목 교체(멱등 병합)인지.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Added(usize),
    /// 동일 idem_key 항목을 제자리 교체했다(큐 길이 불변).
    Merged(usize),
}

impl EnqueueOutcome {
    pub fn depth(&self) -> usize {
        match self {
            EnqueueOutcome::Added(d) | EnqueueOutcome::Merged(d) => *d,
        }
    }
    pub fn merged(&self) -> bool {
        matches!(self, EnqueueOutcome::Merged(_))
    }
}

/// ★E5: 병합이 성립하기 위해 필요한 **최소 잔여 수명**(초). 이보다 적게 남은 항목에 병합을
/// 걸면 갱신본이 배달 기회를 한 번도 못 받고 이관된다 — 그때는 병합하지 않고 신규 발신으로
/// 강등한다(아래 `merge_fits_in_remaining_life` 참조).
pub(crate) const MERGE_MIN_GRACE_SECS: f64 = 300.0;

/// 이 항목에 실제로 걸리는 TTL(초). `0` = 만기 대상 아님(면제·전면 롤백 포함).
///
/// 등급 분기는 `QueueItem::effective_ttl` **한 곳뿐**이다 — 종전에는 여기에 사본 `match` 가
/// 있어서, 만기 판정(state)과 잔여 수명 계산(여기)이 서로 다른 등급표를 볼 수 있었다.
fn effective_ttl_secs(item: &QueueItem) -> f64 {
    item.effective_ttl(crate::governance::queue_ttls())
}

/// 병합이 **잔여 수명 안에 들어가는가**(순수 판정).
///
/// ★E5 재수정(0.13.21 최종 재검증). 종전 구현은 `max(승계, now - ttl + 유예)` 로 바닥을
/// 끌어올렸는데, 그 바닥이 `now` 와 함께 **전진**한다는 게 문제였다: 300s 보다 자주 갱신되는
/// 키는 매 병합마다 "만기까지 300s" 로 복원되어 **영구 잔존**했고, `ttl ≤ 300` 설정에서는
/// 바닥이 사실상 `now` 라 나이가 통째로 리셋됐다(B2 가 없앤 불멸의 재발). "불멸은 재발하지
/// 않는다"던 종전 주석은 거짓이었다.
///
/// 새 의미론은 **병합은 잔여 수명 안에서만**이다:
/// - `remaining ≥ MERGE_MIN_GRACE_SECS` → 제자리 교체 + 나이 **순수 승계**(바닥 보정 없음).
/// - `remaining < MERGE_MIN_GRACE_SECS`(기만료 포함) → 병합하지 않는다. 구 항목은
///   dead-letter(superseded)로 이관하고 갱신본은 **신규 발신**으로 처리한다 — 수명이 다한
///   주제의 갱신은 새 발신이므로 소프트캡·하드캡 역압을 다시 받는 것이 정직하다.
///
/// 불변식: **나이는 절대 연장되지 않는다.** 어떤 병합 반복으로도 한 항목의 총 대기 수명이
/// 자기 등급 TTL 을 넘지 못한다(승계는 나이를 그대로 두고, 못 넘길 상황에서는 병합 자체가
/// 성립하지 않는다). 만기 임박 갱신은 "젊어지기"가 아니라 **신규 발신으로 강등**된다.
///
/// TTL 0(면제·전면 롤백)은 잔여 수명이 무한이므로 항상 승계다.
fn merge_fits_in_remaining_life(fresh: &QueueItem, inherited: f64, now: f64) -> bool {
    let ttl = effective_ttl_secs(fresh);
    if ttl <= 0.0 {
        return true; // 만기 대상이 아니면 잴 잔여 수명 자체가 없다(무한).
    }
    ttl - (now - inherited) >= MERGE_MIN_GRACE_SECS
}

/// 같은 키 항목을 찾은 결과(임계영역 밖으로 들고 나오는 값).
enum MergeSite {
    /// 같은 키 없음 — 평범한 신규 적재.
    Absent,
    /// 잔여 수명 충분 — 제자리 교체 완료(나이 순수 승계).
    Inherited { idx: usize, old: QueueItem, depth: usize },
    /// 잔여 수명 부족 — 구 항목을 큐에서 적출했다. 갱신본은 신규 발신으로 처리한다.
    Demoted { idx: usize, old: QueueItem },
}

/// ★E5 재수정: 만기 임박 키의 갱신을 **신규 발신으로 강등**한다.
///
/// 구 항목은 superseded 로 원장에 남기고(무손실), 갱신본은 `enqueue_item` 에 넘겨 소프트캡·
/// 하드캡·통계·이벤트·WAL 을 **정상 경로 그대로** 통과시킨다. 수명이 다한 주제의 갱신은
/// 실질적으로 새 발신이므로 역압을 다시 받는 것이 정직하다 — 종전처럼 병합으로 처리하면
/// 큐가 아무리 혼잡해도 키 하나만 있으면 역압을 무한히 우회할 수 있었다.
///
/// 원장 기록에 실패하면 적출을 **되돌린다**(원장 없는 삭제 금지 — 구·신 둘 다 보존).
fn demote_to_fresh_send(
    daemon: &Arc<Daemon>,
    surface: &Arc<Surface>,
    idx: usize,
    old: QueueItem,
    fresh: QueueItem,
) -> Result<EnqueueOutcome, EnqueueError> {
    let role = surface.role.lock().unwrap().clone();
    match record_dead_letter(
        daemon,
        surface.id,
        role.as_deref(),
        &old,
        DeadLetterReason::Superseded,
    ) {
        Ok(p) => {
            let ttl = effective_ttl_secs(&fresh);
            let remaining = ttl - (crate::state::now_epoch() - old.enqueued_at);
            daemon.bus.publish(
                queue_events::MERGE_DEMOTED,
                queue_events::CATEGORY,
                Some(surface.id),
                json!({"idem_key": fresh.idem_key, "seq": fresh.seq,
                       "superseded_seq": old.seq,
                       "merged_count": old.merged_count,
                       "ttl_secs": ttl, "remaining_secs": remaining,
                       "grace_secs": MERGE_MIN_GRACE_SECS,
                       "origin_class": fresh.origin.class(),
                       "bytes": fresh.bytes(),
                       "dead_letter": p.display().to_string(),
                       "hint": "구 항목의 잔여 수명이 최소 유예 미만이라 병합하지 않았다 — \
                                구 항목은 superseded 로 원장에 있고 갱신본은 신규 발신으로 \
                                정책(소프트캡·하드캡)을 다시 통과한다. 나이 연장은 없다."}),
            );
            // 적출 자체가 큐 변경이다 — 갱신본이 정책에 거부되면 enqueue_item 은 영속하지
            // 않으므로, 여기서 먼저 영속해야 재시작 시 이관된 구 항목이 되살아나지 않는다.
            daemon.persist_queue_state();
            enqueue_item(daemon, surface, fresh).map(EnqueueOutcome::Added)
        }
        Err(_) => {
            {
                let mut q = surface.pending_queue.lock().unwrap();
                let at = idx.min(q.len());
                q.insert(at, old);
            }
            enqueue_item(daemon, surface, fresh).map(EnqueueOutcome::Added)
        }
    }
}

/// C4 멱등 병합 + 적재. `idem_key`가 있고 대상 큐에 같은 키 항목이 있으면 **제자리 교체**한다.
///
/// 제자리 교체인 이유(D5): 키의 의미는 "이 주제의 최신 상태"다. 갱신할 때마다 뒤로 밀면
/// 자주 갱신하는 주제일수록 배달이 늦어지는 역전이 생긴다.
///
/// 교체 항목은 **fresh seq**를 갖는다(Sim3-1): 배달 임계구역이 head를 복제한 직후 교체가
/// 일어나도 `pop_delivered_head`의 seq 대조가 구/신 항목을 정확히 구분한다 — 구 항목은
/// 배달 완료되고 신 항목은 pop되지 않아 다음 틱에 배달된다(소실·중복 pop 없음).
///
/// 밀려나는 구 텍스트는 dead-letter(`superseded`)에 남는다 — **병합조차 정보를 버리지 않는다**.
/// 원장 기록에 실패하면 병합을 되돌려 구·신 항목을 **둘 다** 보존한다(무손실 우선).
///
/// ★E5 재수정: 같은 키가 있어도 **잔여 수명이 300s 미만이면 병합하지 않는다**
/// (`merge_fits_in_remaining_life`). 구 항목은 superseded 로 이관하고 갱신본은 신규 발신으로
/// 강등해 소프트캡·하드캡을 다시 통과시킨다 — 그래야 자주 갱신되는 키가 나이 리셋으로
/// 불멸이 되는 경로가 닫힌다. 강등분이 정책에 거부되면 발신자는 **정상 거부**를 받는다
/// (구 항목은 이미 원장에 있으므로 무손실).
///
/// 키가 없으면 어떤 병합·중복 억제도 하지 않는다: 동일 문자열 재전송은 정당한 패턴이다
/// (PATH 수복 재시도·boot wake ×4). 억제는 발신자의 명시 선언이 있을 때만.
pub fn enqueue_or_merge(
    daemon: &Arc<Daemon>,
    surface: &Arc<Surface>,
    item: QueueItem,
) -> Result<EnqueueOutcome, EnqueueError> {
    let Some(key) = item.idem_key.clone() else {
        return enqueue_item(daemon, surface, item).map(EnqueueOutcome::Added);
    };

    // ── 임계영역: 같은 키 탐색 → 제자리 교체(승계) 또는 구 항목 적출(강등) ──
    let found = {
        let mut q = surface.pending_queue.lock().unwrap();
        match q.iter().position(|i| i.idem_key.as_deref() == Some(key.as_str())) {
            Some(idx) => {
                let old = q[idx].clone();
                let now = crate::state::now_epoch();
                if merge_fits_in_remaining_life(&item, old.enqueued_at, now) {
                    let mut fresh = item.clone();
                    fresh.merged_count = old.merged_count + 1;
                    // ★B2(0.13.21): 나이(enqueued_at)는 **구 항목 것을 물려받는다**. 종전에는 신
                    // 항목의 현재 시각이 그대로 실려서, 자주 갱신되는 키일수록 TTL 시계가 매번
                    // 0으로 리셋됐다 — 즉 병합 항목은 **영구 불멸**이었고 TTL(C3) 상한이 통째로
                    // 면제됐다. 나이의 의미는 "이 주제의 첫 발신이 언제부터 배달을 기다렸나"이므로
                    // 원 발신 시각이 정본이다(본문·important 등 나머지 필드는 신 항목 우선).
                    //
                    // ★E5 재수정: 승계는 **순수**하다 — 바닥 보정을 하지 않는다. 종전의
                    // `max(승계, now - ttl + 300)` 보정은 바닥이 now 와 함께 전진해서, 300s 보다
                    // 자주 갱신되는 키를 영구 잔존시키고 ttl≤300 에서는 나이를 통째로 리셋했다.
                    // 지금은 "승계로 감당되는 경우"에만 이 가지에 들어오므로 보정할 것이 없다.
                    fresh.enqueued_at = old.enqueued_at;
                    q[idx] = fresh;
                    MergeSite::Inherited { idx, old, depth: q.len() }
                } else {
                    // 잔여 수명 부족 — 병합 불가. 구 항목을 큐에서 적출해 원장으로 보내고
                    // 갱신본은 신규 발신 경로로 넘긴다(정책 재통과).
                    let old = q.remove(idx).expect("position 이 가리킨 자리");
                    MergeSite::Demoted { idx, old }
                }
            }
            None => MergeSite::Absent,
        }
    };

    let (idx, old, depth) = match found {
        MergeSite::Absent => return enqueue_item(daemon, surface, item).map(EnqueueOutcome::Added),
        MergeSite::Demoted { idx, old } => {
            return demote_to_fresh_send(daemon, surface, idx, old, item)
        }
        MergeSite::Inherited { idx, old, depth } => (idx, old, depth),
    };

    let role = surface.role.lock().unwrap().clone();
    match record_dead_letter(
        daemon,
        surface.id,
        role.as_deref(),
        &old,
        DeadLetterReason::Superseded,
    ) {
        Ok(p) => {
            // ★B2 통계 의미론(실측 확정): 병합은 **수용된 발신**이다 — total 만 올리고
            // rejected 는 올리지 않는다. `rejected` 는 org.status·surface.list 가 노출하는
            // "규약 위반 스크리닝 신호"라, 거부당한 적 없는 발신자를 여기서 세면 신호가
            // 오염된다(superseded 는 구 항목의 퇴장이지 신 발신의 거부가 아니다).
            // keyless=false 도 사실이다 — 병합은 idem_key 가 있을 때만 일어난다.
            if let Some(sid) = item.origin.sender_surface() {
                bump_stats(daemon, sid, false, false);
            }
            daemon.bus.publish(
                queue_events::MERGED,
                queue_events::CATEGORY,
                Some(surface.id),
                json!({"idem_key": key, "depth": depth,
                       "merged_count": old.merged_count + 1,
                       "seq": item.seq, "superseded_seq": old.seq,
                       "origin_class": item.origin.class(),
                       "bytes": item.bytes(),
                       "dead_letter": p.display().to_string()}),
            );
            daemon.persist_queue_state();
            Ok(EnqueueOutcome::Merged(depth))
        }
        Err(_) => {
            // 원장을 못 남겼다 = 구 텍스트를 버릴 수 없다. 병합을 되돌리고 구 항목을
            // 제자리에 복원한 뒤, 신 항목은 별건으로 적재한다(둘 다 보존 — 무손실 우선).
            {
                let mut q = surface.pending_queue.lock().unwrap();
                let pos = q
                    .iter()
                    .position(|i| i.seq == item.seq)
                    .unwrap_or(idx.min(q.len()));
                if q.get(pos).map(|i| i.seq) == Some(item.seq) {
                    q[pos] = old;
                } else {
                    let at = pos.min(q.len());
                    q.insert(at, old);
                }
            }
            enqueue_item(daemon, surface, item).map(EnqueueOutcome::Added)
        }
    }
}

/// 항목 하나를 대상 큐에 적재한다. 정책 판정·통계·이벤트·WAL 영속을 한 곳에서 수행해
/// 세 enqueue 경로가 서로 다른 규율을 갖는 일(오늘의 결함 원형)을 구조적으로 막는다.
///
/// 락 규율: pending_queue 락은 카운트·캡·push 구간에서만 잡고, 이벤트·dead-letter·
/// `persist_queue_state`는 **락 해제 후** 수행한다(persist는 락 비보유가 전제조건).
pub fn enqueue_item(
    daemon: &Arc<Daemon>,
    surface: &Arc<Surface>,
    item: QueueItem,
) -> Result<usize, EnqueueError> {
    let softcap = agent_softcap();
    let mode = policy_mode();
    let is_agent = item.origin.is_agent();
    let sender = item.origin.sender_surface();
    let keyless = item.idem_key.is_none();

    // ── 임계영역: 카운트 → 캡 판정 → (허용 시) push ──
    let (verdict, agent_depth, depth) = {
        let mut q = surface.pending_queue.lock().unwrap();
        if q.len() >= QUEUE_HARD_CAP {
            drop(q);
            if let Some(sid) = sender {
                bump_stats(daemon, sid, keyless, true);
            }
            return Err(EnqueueError::HardCap);
        }
        let agent_depth = q.iter().filter(|i| i.origin.is_agent()).count();
        let verdict = decide(is_agent, agent_depth, softcap, mode);
        if verdict == PolicyVerdict::Reject {
            (verdict, agent_depth, q.len())
        } else {
            q.push_back(item.clone());
            (verdict, agent_depth, q.len())
        }
    };

    if let Some(sid) = sender {
        bump_stats(daemon, sid, keyless, verdict == PolicyVerdict::Reject);
    }

    match verdict {
        PolicyVerdict::Reject => {
            // 거부와 **동시에** 데몬이 전문을 원장에 남긴다(발신자 행동에 의존하지 않는다).
            let role = surface.role.lock().unwrap().clone();
            let dl = record_dead_letter(
                daemon,
                surface.id,
                role.as_deref(),
                &item,
                DeadLetterReason::SoftcapRejected,
            );
            daemon.bus.publish(
                queue_events::REJECTED,
                queue_events::CATEGORY,
                Some(surface.id),
                json!({"mode": "enforce", "agent_depth": agent_depth, "softcap": softcap,
                       "origin_class": item.origin.class(), "role": item.origin.role(),
                       "bytes": item.bytes(),
                       "dead_letter": dl.as_ref().ok().map(|p| p.display().to_string()),
                       "dead_letter_error": dl.as_ref().err()}),
            );
            match dl {
                Ok(p) => Err(EnqueueError::Softcap {
                    dead_letter: Some(p.display().to_string()),
                    agent_depth,
                    softcap,
                }),
                // 원장 기록 실패 = 원장 없는 거부는 금지. 항목을 수용하고 통과시킨다.
                Err(_) => {
                    let depth = {
                        let mut q = surface.pending_queue.lock().unwrap();
                        q.push_back(item.clone());
                        q.len()
                    };
                    publish_enqueued(daemon, surface.id, &item, depth);
                    daemon.persist_queue_state();
                    Ok(depth)
                }
            }
        }
        PolicyVerdict::LogOnly => {
            daemon.bus.publish(
                queue_events::REJECTED,
                queue_events::CATEGORY,
                Some(surface.id),
                json!({"mode": "log", "agent_depth": agent_depth, "softcap": softcap,
                       "origin_class": item.origin.class(), "role": item.origin.role(),
                       "bytes": item.bytes(), "admitted": true,
                       "hint": "관찰 모드(CYS_QUEUE_POLICY_MODE=log) — 적재는 허용됐다. \
                                기본값은 차단(enforce)이며 env 를 지우면 복귀한다"}),
            );
            publish_enqueued(daemon, surface.id, &item, depth);
            daemon.persist_queue_state();
            Ok(depth)
        }
        PolicyVerdict::Allow => {
            publish_enqueued(daemon, surface.id, &item, depth);
            daemon.persist_queue_state();
            Ok(depth)
        }
    }
}

fn publish_enqueued(daemon: &Arc<Daemon>, sid: u64, item: &QueueItem, depth: usize) {
    daemon.bus.publish(
        queue_events::ENQUEUED,
        queue_events::CATEGORY,
        Some(sid),
        json!({"bytes": item.bytes(), "depth": depth,
               "origin_class": item.origin.class(),
               "verified": item.origin.is_agent(),
               "idem": item.idem_key.is_some(),
               "kind": item.kind.as_str(), "seq": item.seq,
               "from": item.origin.role()}),
    );
}

/// 큐에서 가장 오래된 항목의 나이(초) — `surface.list` 노출용(D10).
pub fn oldest_age_secs(surface: &Surface) -> Option<f64> {
    let now = crate::state::now_epoch();
    surface
        .pending_queue
        .lock()
        .unwrap()
        .iter()
        .map(|i| now - i.enqueued_at)
        .fold(None, |acc: Option<f64>, a| {
            Some(acc.map_or(a, |m: f64| m.max(a)))
        })
}

/// 발신 origin 판별(D2). **근거는 커널 peer pid로 해석된 `verified_from` 뿐이다** —
/// 자기신고 `from`도, 자기신고 `human`도 판별에 쓰지 않는다.
///
/// ★자기신고 `human` 제거(D2 문언 준수): 종전에는 `human:true` 한 줄이면 어떤 발신자든
/// Human = 정책 면제로 분류됐다. 신원 위조가 곧 정책 우회인 구조는 "self-report 불신"이라는
/// D2의 전제를 스스로 무너뜨린다. `QueueOrigin::Human` variant 는 **WAL v1/v2 역호환**을 위해
/// 타입으로만 남고, 신규 분류는 더 이상 이 variant 를 만들지 않는다(GUI 큐잉 발신은 호출부가
/// System 라벨 `"gui"`로 세분화 — 소프트캡 등급은 System 그대로다).
/// ★E4: 그 라벨은 **GUI 전용 TTL 등급의 근거**이기도 하다(`QueueItem::effective_ttl`) — 실사람
/// 입력이 이 경로로만 들어오므로, 라벨을 잃으면 사람이 친 메시지가 24h 가 아니라 4h 뒤에
/// dead-letter 로 이관된다. 반대로 이 라벨이 **무기한 면제**를 주지는 않는다(E4 재수정):
/// 라벨의 근거가 자기신고인 이상 무기한은 위조 대상이 되므로 유계 장주기로 둔다.
///
/// ★Agent 판정 확장: `agent_meta` 는 `cys launch-agent` 로 기동한 노드만 갖는다. 그런데
/// CLAUDE.md §6 정식 절차(`cys new-surface` → 그 pane 에서 직접 `agy`/`codex` 실행)로 편입된
/// 리뷰어 pane 은 meta 가 없어 **정책 밖**으로 새어나갔다 — 정확히 그 노드들이 폭주 리스크의
/// 주체인데도. 좌석 사실(`seat_cache == Occupied`, 자손 프로세스 존재)을 두 번째 근거로 받아
/// 수동 기동 에이전트도 정책 안으로 들인다. Unknown(프로브 미도달)은 확장하지 않는다 —
/// 판정 실패를 근거로 삼으면 맨 셸 자동화를 오차단한다.
pub fn derive_origin(daemon: &Daemon, verified_from: Option<u64>) -> QueueOrigin {
    let Some(sid) = verified_from else {
        // 조상추적 미해소(외부 프로세스·데몬 내부) — 위협 모델상 정책 밖.
        return QueueOrigin::system("anonymous");
    };
    if let Some(s) = daemon.get_surface(sid) {
        let registered = s.agent_meta.lock().unwrap().is_some();
        let seated = crate::governance::SeatState::from_u8(
            s.seat_cache.load(std::sync::atomic::Ordering::Relaxed),
        ) == crate::governance::SeatState::Occupied;
        if registered || seated {
            return QueueOrigin::Agent {
                surface: sid,
                role: s.role.lock().unwrap().clone(),
            };
        }
    }
    // verified 이지만 에이전트도 좌석 점유도 아님(맨 셸 pane) — 정책 대상 아님.
    QueueOrigin::system("pane")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T2-a: 소프트캡 판정 순수 함수 — origin 면제·0=off·모드별 분기.
    #[test]
    fn softcap_decision_matrix() {
        // System/Human origin은 어떤 depth·모드에서도 정책 밖이다.
        assert_eq!(
            decide(false, 999, 25, PolicyMode::Enforce),
            PolicyVerdict::Allow,
            "System/Human origin에 소프트캡을 적용하면 부트 wake·시스템 통지가 오차단된다"
        );
        // softcap=0 = 비활성.
        assert_eq!(decide(true, 999, 0, PolicyMode::Enforce), PolicyVerdict::Allow);
        // 임계 미만 통과.
        assert_eq!(decide(true, 24, 25, PolicyMode::Enforce), PolicyVerdict::Allow);
        // 임계 도달 — 모드별 분기(기본 log는 적재 허용).
        assert_eq!(decide(true, 25, 25, PolicyMode::Log), PolicyVerdict::LogOnly);
        assert_eq!(decide(true, 25, 25, PolicyMode::Enforce), PolicyVerdict::Reject);
    }

    /// T2-b/★B1: 모드 파싱 — 미설정·오타·대소문자. **기본은 차단(enforce)** 이고
    /// 관찰 모드는 `log` 를 명시했을 때만 성립한다(오타가 억제를 조용히 끄면 안 된다).
    #[test]
    fn policy_mode_defaults_to_enforce_and_log_is_opt_in() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        // 미설정 = 기본 = enforce. 이게 Log 였던 동안 거부 경로는 실배포 도달 불능이었다.
        std::env::remove_var("CYS_QUEUE_POLICY_MODE");
        assert_eq!(
            policy_mode(),
            PolicyMode::Enforce,
            "env 미설정 기본값이 Enforce 가 아니면 소프트캡 거부가 다시 死코드가 된다"
        );
        let cases = [
            ("", PolicyMode::Enforce),
            ("log", PolicyMode::Log),
            ("LOG", PolicyMode::Log),
            (" log ", PolicyMode::Log),
            ("garbage", PolicyMode::Enforce), // 오타는 fail-safe 쪽(차단)으로 떨어진다
            ("ENFORCE", PolicyMode::Enforce),
            (" enforce ", PolicyMode::Enforce),
        ];
        for (v, want) in cases {
            std::env::set_var("CYS_QUEUE_POLICY_MODE", v);
            assert_eq!(policy_mode(), want, "mode 파싱: {v:?}");
        }
        std::env::remove_var("CYS_QUEUE_POLICY_MODE");
    }

    /// T2-c: 사유 문자열은 원장 계약이다 — 팩 도구(javis_deadletter)가 이 값으로 분류한다.
    #[test]
    fn dead_letter_reason_strings_are_stable() {
        assert_eq!(DeadLetterReason::Ttl.as_str(), "ttl");
        assert_eq!(DeadLetterReason::SoftcapRejected.as_str(), "softcap_rejected");
        assert_eq!(DeadLetterReason::Superseded.as_str(), "superseded");
        assert_eq!(DeadLetterReason::WakeupCap.as_str(), "wakeup_cap");
        assert_eq!(DeadLetterReason::ClearedByOperator.as_str(), "cleared_by_operator");
        // 신설 3종 — self clear·surface 종료 2경로의 무음 삭제를 없앤 사유들.
        assert_eq!(DeadLetterReason::Cleared.as_str(), "cleared");
        assert_eq!(DeadLetterReason::SurfaceClosed.as_str(), "surface_closed");
        assert_eq!(DeadLetterReason::ProcessExited.as_str(), "process_exited");
    }

    /// A5: 회전본 보존기간 prune — 오래된 회전본만 지우고 **현행 파일은 절대 건드리지 않는다**.
    /// mtime 조작이 필요해 unix 한정(prune 로직 자체는 OS 무관).
    #[cfg(unix)]
    #[test]
    fn rotated_dead_letters_are_pruned_after_retention() {
        let dir = std::env::temp_dir().join(format!(
            "cys-dlprune-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let cur = dir.join(DEAD_LETTER_FILE);
        let old = dir.join("dead-letters.1000000000.jsonl");
        let fresh = dir.join("dead-letters.2000000000.jsonl");
        let other = dir.join("schedule_state.json");
        for p in [&cur, &old, &fresh, &other] {
            std::fs::write(p, "{}\n").unwrap();
        }
        // 현행 파일도 함께 '오래된' 것으로 만든다 — 그래도 살아남아야 계약이다.
        let ancient = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86_400);
        for p in [&cur, &old, &other] {
            filetime_set(p, ancient);
        }

        let n = prune_rotated_dead_letters(&dir, DEAD_LETTER_RETAIN_DAYS);
        assert_eq!(n, 1, "보존기간 초과 회전본 1건만 지워야 한다");
        assert!(!old.exists(), "40일 지난 회전본이 남았다");
        assert!(fresh.exists(), "보존기간 내 회전본을 지웠다");
        assert!(cur.exists(), "★현행 원장을 지웠다 — 무손실 위반");
        assert!(other.exists(), "무관 파일을 지웠다");

        // retain_days=0 = 비활성(안전판).
        assert_eq!(prune_rotated_dead_letters(&dir, 0), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★B1/B2 (0.13.21 hotfix) — 기본 Enforce 배선 · 병합의 TTL 보존·통계 정직성
    // ─────────────────────────────────────────────────────────────────────────

    fn hotfix_daemon(tag: &str) -> (Arc<Daemon>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cys-qhot-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        (Daemon::new(dir.join("cysd.sock")), dir)
    }

    fn hotfix_surface(d: &Arc<Daemon>, role: Option<&str>) -> Arc<Surface> {
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, role.map(|r| r.into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s
    }

    fn agent_origin() -> QueueOrigin {
        QueueOrigin::Agent { surface: 4242, role: Some("worker-1".into()) }
    }

    /// ★E6(적대 리뷰 REVISE-8): env 원복을 **Drop 가드**로 한다. 함수 말미의 `remove_var` 는
    /// 테스트가 assert 로 패닉하면 실행되지 않아, 설정된 env 가 같은 프로세스의 다른 테스트로
    /// 새어 나간다(락을 잡아도 값 자체는 남는다 — flake 의 실제 경로).
    /// governance 의 `TtlEnv` 와 같은 패턴이며, 여기서는 여러 키를 한 가드로 다룬다.
    struct QueueEnvGuard(&'static [&'static str]);
    impl QueueEnvGuard {
        /// `(키, 값)` 설정 + 원복 대상 키 목록. 값이 `None` 이면 "미설정 상태로 시작"이다.
        fn set(pairs: &[(&'static str, Option<&str>)], restore: &'static [&'static str]) -> Self {
            for (k, v) in pairs {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
            QueueEnvGuard(restore)
        }
    }
    impl Drop for QueueEnvGuard {
        fn drop(&mut self) {
            for k in self.0 {
                std::env::remove_var(k);
            }
        }
    }

    /// ★B1: env 를 **아무것도 걸지 않은** 기본 상태에서 소프트캡 초과분이 실제로 거부되는가.
    /// 종전 기본(Log)에서는 이 경로가 실배포 도달 불능이었다 — 그래서 순수 `decide()` 단위
    /// 테스트가 아니라 enqueue 배선까지 통과시켜 핀한다.
    #[test]
    fn default_env_enforces_softcap_at_enqueue() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_POLICY_MODE", None), // 기본값(=Enforce) 그대로
                ("CYS_QUEUE_AGENT_SOFTCAP", Some("2")),
            ],
            &["CYS_QUEUE_POLICY_MODE", "CYS_QUEUE_AGENT_SOFTCAP"],
        );
        let (d, dir) = hotfix_daemon("default-enforce");
        let target = hotfix_surface(&d, Some("master"));

        assert!(enqueue_item(&d, &target, QueueItem::text("a".into(), agent_origin())).is_ok());
        assert!(enqueue_item(&d, &target, QueueItem::text("b".into(), agent_origin())).is_ok());
        let r = enqueue_item(&d, &target, QueueItem::text("초과분".into(), agent_origin()));
        assert!(
            matches!(r, Err(EnqueueError::Softcap { .. })),
            "env 미설정 기본에서 소프트캡이 차단하지 않으면 억제 계층이 다시 死코드다: {r:?}"
        );
        assert_eq!(target.pending_queue.lock().unwrap().len(), 2);

        // 롤백 스위치: log 를 명시하면 종전 관찰 동작(적재 허용)으로 되돌아간다.
        std::env::set_var("CYS_QUEUE_POLICY_MODE", "log");
        assert!(
            enqueue_item(&d, &target, QueueItem::text("관찰 통과".into(), agent_origin())).is_ok(),
            "CYS_QUEUE_POLICY_MODE=log 폴백이 살아 있어야 런타임 롤백이 가능하다"
        );
        assert_eq!(target.pending_queue.lock().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 테스트용 3등급 TTL 묶음(세 등급 동일).
    fn ttls(secs: f64) -> crate::state::QueueTtls {
        crate::state::QueueTtls { agent: secs, system: secs, gui: secs }
    }

    fn hotfix_dead_letters(dir: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(state_dir(&dir.join("cysd.sock")).join(DEAD_LETTER_FILE))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("유효 JSON"))
            .collect()
    }

    /// 큐에서 `key` 항목의 나이(초). 없으면 None.
    fn age_of(s: &Arc<Surface>, key: &str) -> Option<f64> {
        let now = crate::state::now_epoch();
        s.pending_queue
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.idem_key.as_deref() == Some(key))
            .map(|i| now - i.enqueued_at)
    }

    /// 큐의 `key` 항목을 `secs` 만큼 **더 늙힌다**(시간 경과 시뮬레이션 — 실시간 대기 없이
    /// 반복 병합의 누적 효과를 재현하기 위한 유일한 수단).
    fn age_item(s: &Arc<Surface>, key: &str, secs: f64) {
        let mut q = s.pending_queue.lock().unwrap();
        for i in q.iter_mut() {
            if i.idem_key.as_deref() == Some(key) {
                i.enqueued_at -= secs;
            }
        }
    }

    /// ★B2: 병합은 나이를 물려받는다 — 갱신이 잦아도 TTL 시계는 리셋되지 않는다(불멸 제거).
    /// ★E5 재수정: 승계는 **순수**하다(바닥 보정 없음) — 잔여 수명이 넉넉한 이 구간에서
    /// 승계값이 그대로 실리는지 핀한다.
    /// 함께 핀: 병합 발신은 total 로만 세고 rejected 는 오르지 않는다(스크리닝 신호 오염 방지).
    #[test]
    fn merge_preserves_enqueued_at_and_counts_as_accepted_send() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        // 이 테스트의 주제는 병합이다 — 소프트캡은 끄고, TTL 은 유예(300s)보다 충분히 길게 잡아
        // 잔여 수명이 넉넉한 구간(병합 성립)에서 승계 자체를 핀한다.
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_AGENT_SOFTCAP", Some("0")),
                ("CYS_QUEUE_TTL_SECS", Some("3600")),
            ],
            &["CYS_QUEUE_AGENT_SOFTCAP", "CYS_QUEUE_TTL_SECS"],
        );
        let (d, dir) = hotfix_daemon("merge-age");
        let s = hotfix_surface(&d, Some("master"));

        let mut old = QueueItem::text("상태 v1".into(), agent_origin());
        old.idem_key = Some("k".into());
        old.enqueued_at = crate::state::now_epoch() - 300.0; // 5분 전 발신
        let origin_at = old.enqueued_at;
        assert!(enqueue_or_merge(&d, &s, old).is_ok());

        let mut fresh = QueueItem::text("상태 v2".into(), agent_origin());
        fresh.idem_key = Some("k".into());
        let out = enqueue_or_merge(&d, &s, fresh).expect("병합");
        assert!(out.merged(), "같은 키는 제자리 병합이어야 한다");

        {
            let q = s.pending_queue.lock().unwrap();
            assert_eq!(q.len(), 1);
            assert_eq!(q[0].text, "상태 v2", "본문은 신 항목 우선(기존 의미 보존)");
            assert_eq!(q[0].merged_count, 1);
            assert!(
                (q[0].enqueued_at - origin_at).abs() < 0.001,
                "병합이 나이를 리셋하면(또는 바닥 보정으로 당기면) TTL 이 우회된다"
            );
            // 나이가 보존됐으므로 만기 판정도 원 발신 시각 기준이다(TTL 60s < 300s 경과).
            assert!(
                q[0].is_expired(crate::state::now_epoch(), ttls(60.0)),
                "보존된 나이가 만기 판정에 실제로 반영돼야 한다"
            );
        }

        let st = daemon_stats(&d, 4242);
        assert_eq!(st.total, 2, "발신 2건(신규 1 + 병합 1) 모두 수용된 발신이다");
        assert_eq!(st.rejected, 0, "병합을 거부로 세면 규약 위반 스크리닝 신호가 오염된다");
        assert_eq!(st.keyless, 0, "병합은 키가 있을 때만 일어난다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E5 재수정 — 순수 판정 핀: **병합은 잔여 수명 안에서만** 성립한다.
    ///
    /// 종전 구현(`max(승계, now - ttl + 300)`)은 바닥이 `now` 와 함께 전진해, 300s 미만
    /// 간격으로 갱신되는 키를 영구 잔존시켰고 `ttl ≤ 300` 에서는 나이를 통째로 리셋했다.
    /// 지금은 잔여가 유예 미만이면 **병합 자체가 거부**되고 신규 발신으로 강등된다.
    #[test]
    fn merge_fits_only_within_remaining_life() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[("CYS_QUEUE_TTL_SECS", Some("600"))],
            &["CYS_QUEUE_TTL_SECS"],
        );
        let now = crate::state::now_epoch();
        let item = QueueItem::text("x".into(), agent_origin());
        // 잔여 500s / 300s(경계 포함) → 병합 성립.
        assert!(merge_fits_in_remaining_life(&item, now - 100.0, now));
        assert!(merge_fits_in_remaining_life(&item, now - 300.0, now), "경계(=유예)는 성립");
        // 잔여 299s / 기만료 → 병합 불가(강등).
        assert!(!merge_fits_in_remaining_life(&item, now - 301.0, now));
        assert!(!merge_fits_in_remaining_life(&item, now - 99_999.0, now), "기만료도 강등");
    }

    /// ★E5 재수정 — 만기 임박 갱신은 **신규 발신으로 강등**된다(병합 아님).
    ///
    /// 구 항목은 superseded 로 원장에 남고, 갱신본은 `enqueue_item` 정상 경로로 들어가
    /// 나이가 0 에서 다시 시작한다. 이때 **연장된 것은 아무것도 없다** — 구 항목은 자기
    /// 수명대로 큐를 떠났고 새 항목은 별개의 발신이다.
    #[test]
    fn expiring_merge_is_demoted_to_a_fresh_send() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_AGENT_SOFTCAP", Some("0")),
                ("CYS_QUEUE_TTL_SECS", Some("600")),
            ],
            &["CYS_QUEUE_AGENT_SOFTCAP", "CYS_QUEUE_TTL_SECS"],
        );
        let (d, dir) = hotfix_daemon("merge-demote");
        let s = hotfix_surface(&d, Some("master"));

        let mut old = QueueItem::text("임박 v1".into(), agent_origin());
        old.idem_key = Some("tight".into());
        old.enqueued_at = crate::state::now_epoch() - 590.0; // 잔여 10s < 유예 300s
        enqueue_or_merge(&d, &s, old).expect("적재");

        let mut fresh = QueueItem::text("임박 v2".into(), agent_origin());
        fresh.idem_key = Some("tight".into());
        let out = enqueue_or_merge(&d, &s, fresh).expect("강등 적재");
        assert!(
            !out.merged(),
            "잔여 수명이 유예 미만인데 병합하면(=시계를 당기면) 불멸 경로가 다시 열린다"
        );

        {
            let q = s.pending_queue.lock().unwrap();
            assert_eq!(q.len(), 1, "구 항목은 적출되고 갱신본만 남는다");
            assert_eq!(q[0].text, "임박 v2");
            assert_eq!(q[0].merged_count, 0, "병합이 아니므로 병합 카운트도 오르지 않는다");
            let age = crate::state::now_epoch() - q[0].enqueued_at;
            assert!(age < 2.0, "강등분은 신규 발신이라 나이가 0 에서 시작한다(실측 {age}s)");
        }
        let dl = hotfix_dead_letters(&dir);
        assert_eq!(dl.len(), 1, "구 항목은 무음 삭제가 아니라 원장 이관이다");
        assert_eq!(dl[0]["text"], serde_json::json!("임박 v1"), "전문 무손실");
        assert_eq!(dl[0]["reason"], serde_json::json!("superseded"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E5 재수정 — **반복 병합으로 총수명이 TTL 을 넘지 못한다**(불멸 재발 차단 본핀).
    ///
    /// 유예(300s)보다 짧은 간격으로 계속 갱신해도, 병합이 성립하는 동안 나이는 **연장되지
    /// 않고**(젊어지지 않고) 계속 늙는다. 잔여가 유예 미만이 되는 순간 병합은 강등으로
    /// 바뀌어 구 항목이 큐를 떠난다. 종전 구현에서는 이 루프에서 나이가 매번 300s 로 되감겨
    /// 항목이 영원히 큐에 남았다.
    #[test]
    fn repeated_merges_never_extend_total_lifetime() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_AGENT_SOFTCAP", Some("0")),
                ("CYS_QUEUE_TTL_SECS", Some("600")),
            ],
            &["CYS_QUEUE_AGENT_SOFTCAP", "CYS_QUEUE_TTL_SECS"],
        );
        let (d, dir) = hotfix_daemon("merge-repeat");
        let s = hotfix_surface(&d, Some("master"));

        let mut first = QueueItem::text("v0".into(), agent_origin());
        first.idem_key = Some("hot".into());
        enqueue_or_merge(&d, &s, first).expect("적재");

        let mut demotions = 0;
        for round in 1..=6 {
            // 유예(300s)보다 짧은 간격으로 갱신한다 — 종전 구현이 불멸을 만들던 바로 그 패턴.
            age_item(&s, "hot", 100.0);
            let before = age_of(&s, "hot").expect("항목");
            let mut fresh = QueueItem::text(format!("v{round}"), agent_origin());
            fresh.idem_key = Some("hot".into());
            let out = enqueue_or_merge(&d, &s, fresh).expect("적재");
            let after = age_of(&s, "hot").expect("항목");
            if out.merged() {
                assert!(
                    after >= before - 1.0,
                    "병합이 나이를 젊게 만들었다(round {round}: {before}s → {after}s) — \
                     이 되감기가 곧 불멸이다"
                );
            } else {
                demotions += 1;
                assert!(
                    before + MERGE_MIN_GRACE_SECS > 600.0,
                    "잔여가 충분한데 강등했다(round {round}: 나이 {before}s)"
                );
            }
            assert!(
                after <= 600.0,
                "큐에 남은 항목의 나이가 TTL 을 넘겼다(round {round}: {after}s) — \
                 sweep 이 돌기 전에 이미 계약 위반이다"
            );
        }
        assert!(demotions >= 1, "6라운드(600s 경과) 동안 강등이 한 번도 없으면 불멸이다");
        // 강등된 구 항목들은 전부 원장에 있다(무손실).
        let dl = hotfix_dead_letters(&dir);
        assert_eq!(
            dl.iter().filter(|e| e["reason"] == serde_json::json!("superseded")).count(),
            6,
            "병합·강등 어느 쪽이든 밀려난 구 텍스트는 전부 원장에 남는다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E5 재수정 — TTL 이 유예보다 짧아도(`ttl < 300`) **나이 리셋은 없다**.
    ///
    /// 종전 구현에서 이 설정은 바닥이 사실상 `now` 라 병합마다 나이가 통째로 리셋됐다
    /// (원 결함 완전 재발). 지금은 잔여가 늘 유예 미만이므로 **모든 갱신이 강등**된다 —
    /// 구 항목은 원장으로 가고 갱신본은 새 발신으로 자기 TTL 을 처음부터 산다.
    #[test]
    fn short_ttl_below_grace_demotes_instead_of_resetting_age() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_AGENT_SOFTCAP", Some("0")),
                ("CYS_QUEUE_TTL_SECS", Some("120")), // < MERGE_MIN_GRACE_SECS(300)
            ],
            &["CYS_QUEUE_AGENT_SOFTCAP", "CYS_QUEUE_TTL_SECS"],
        );
        let (d, dir) = hotfix_daemon("merge-short-ttl");
        let s = hotfix_surface(&d, Some("master"));

        let mut old = QueueItem::text("짧은 v1".into(), agent_origin());
        old.idem_key = Some("short".into());
        old.enqueued_at = crate::state::now_epoch() - 100.0; // 잔여 20s
        enqueue_or_merge(&d, &s, old).expect("적재");

        let mut fresh = QueueItem::text("짧은 v2".into(), agent_origin());
        fresh.idem_key = Some("short".into());
        let out = enqueue_or_merge(&d, &s, fresh).expect("강등 적재");
        assert!(
            !out.merged(),
            "ttl<유예 에서 병합을 성립시키면 나이가 통째로 리셋된다(원 결함 재발)"
        );
        assert_eq!(
            s.pending_queue.lock().unwrap().len(),
            1,
            "강등은 구 항목 적출 + 신규 적재라 깊이가 늘지 않는다"
        );
        assert_eq!(hotfix_dead_letters(&dir).len(), 1, "구 항목은 원장 이관(무손실)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E5 재수정 — 강등분은 **소프트캡을 다시 통과**한다.
    ///
    /// 종전에는 키 하나만 쥐고 있으면 큐가 아무리 혼잡해도 병합으로 역압을 무한 우회할 수
    /// 있었다. 수명이 다한 주제의 갱신은 실질적으로 새 발신이므로 정책을 다시 받는다.
    /// 거부돼도 무손실이다 — 구 항목은 superseded 로, 갱신본은 softcap_rejected 로 원장에 있다.
    #[test]
    fn demoted_merge_is_subject_to_softcap_again() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_AGENT_SOFTCAP", Some("2")),
                ("CYS_QUEUE_TTL_SECS", Some("600")),
                ("CYS_QUEUE_POLICY_MODE", None), // 기본 enforce
            ],
            &["CYS_QUEUE_AGENT_SOFTCAP", "CYS_QUEUE_TTL_SECS", "CYS_QUEUE_POLICY_MODE"],
        );
        let (d, dir) = hotfix_daemon("merge-demote-softcap");
        let s = hotfix_surface(&d, Some("master"));

        // 소프트캡(2)을 채우는 Agent 항목 2건 + 만기 임박한 키 항목 1건.
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(QueueItem::text("혼잡 1".into(), agent_origin()));
            q.push_back(QueueItem::text("혼잡 2".into(), agent_origin()));
            let mut old = QueueItem::text("임박 v1".into(), agent_origin());
            old.idem_key = Some("tight".into());
            old.enqueued_at = crate::state::now_epoch() - 590.0; // 잔여 10s
            q.push_back(old);
        }

        let mut fresh = QueueItem::text("임박 v2".into(), agent_origin());
        fresh.idem_key = Some("tight".into());
        let r = enqueue_or_merge(&d, &s, fresh);
        assert!(
            matches!(r, Err(EnqueueError::Softcap { .. })),
            "강등분이 소프트캡을 우회하면 키 하나로 역압을 무한 우회할 수 있다: {r:?}"
        );
        assert_eq!(
            s.pending_queue.lock().unwrap().len(),
            2,
            "구 항목은 이관됐고 갱신본은 거부됐다 — 남는 것은 혼잡분 2건"
        );
        let dl = hotfix_dead_letters(&dir);
        let reasons: Vec<&str> = dl.iter().filter_map(|e| e["reason"].as_str()).collect();
        assert_eq!(
            reasons,
            vec!["superseded", "softcap_rejected"],
            "거부돼도 무손실이어야 한다(구 항목·갱신본 둘 다 원장)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E5: 잔여 수명 판정은 **면제 항목을 늘 통과시킨다** — important 는 만기 대상이 아니라
    /// 잴 잔여 수명 자체가 없다(아무리 늙어도 순수 승계 병합).
    /// ★E4 재수정 반영: GUI 는 더 이상 면제가 아니라 **자기 등급(기본 24h)**이므로 여기서
    /// 함께 핀하지 않는다 — GUI 도 잔여 수명이 유예 미만이면 강등된다(아래 별도 핀).
    #[test]
    fn merge_fit_always_true_for_exempt_items() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[("CYS_QUEUE_TTL_SECS", Some("600"))],
            &["CYS_QUEUE_TTL_SECS"],
        );
        let now = crate::state::now_epoch();
        let ancient = now - 99_999.0;
        let mut imp = QueueItem::text("중요".into(), agent_origin());
        imp.important = true;
        assert!(
            merge_fits_in_remaining_life(&imp, ancient, now),
            "important 는 만기 대상이 아니다 — 잔여 수명 무한이므로 늘 순수 승계다"
        );
        // 전면 롤백 스위치(TTL=0)도 같은 이유로 항상 승계다.
        std::env::set_var("CYS_QUEUE_TTL_SECS", "0");
        let plain = QueueItem::text("보통".into(), agent_origin());
        assert!(
            merge_fits_in_remaining_life(&plain, ancient, now),
            "TTL 전면 비활성에서 강등이 일어나면 롤백 스위치가 정책을 바꾸는 셈이다"
        );
    }

    /// ★E4 재수정 — GUI 큐잉은 **면제가 아니라 24h 등급**이므로 잔여 수명 계산의 대상이다.
    /// 종전(면제)이었다면 아무리 늙어도 승계였다.
    #[test]
    fn gui_merge_uses_its_own_bounded_tier() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap();
        let _env = QueueEnvGuard::set(
            &[
                ("CYS_QUEUE_TTL_SECS", Some("600")),
                ("CYS_QUEUE_GUI_TTL_SECS", Some("86400")),
            ],
            &["CYS_QUEUE_TTL_SECS", "CYS_QUEUE_GUI_TTL_SECS"],
        );
        let now = crate::state::now_epoch();
        let gui = QueueItem::text(
            "사람 입력".into(),
            QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI),
        );
        // 하루 안이면 잔여가 넉넉하다 → 승계 병합.
        assert!(merge_fits_in_remaining_life(&gui, now - 3600.0, now), "1h 된 GUI 는 승계");
        // 24h 문턱 직전이면 잔여가 유예 미만 → 강등.
        assert!(
            !merge_fits_in_remaining_life(&gui, now - 86_300.0, now),
            "GUI 가 면제였던 시절의 무조건 승계가 남아 있으면 이 핀이 깨진다"
        );
        // 운영자가 0(면제)을 명시하면 다시 무한 승계다.
        std::env::set_var("CYS_QUEUE_GUI_TTL_SECS", "0");
        assert!(
            merge_fits_in_remaining_life(&gui, now - 99_999_999.0, now),
            "CYS_QUEUE_GUI_TTL_SECS=0 은 면제 선언이다"
        );
    }

    fn daemon_stats(d: &Arc<Daemon>, sid: u64) -> QueueSendStats {
        d.queue_send_stats.lock().unwrap().get(&sid).copied().unwrap_or_default()
    }

    /// mtime 을 과거로 미는 최소 헬퍼(외부 크레이트 무의존 — utimes(2) 직접 호출).
    #[cfg(unix)]
    fn filetime_set(path: &std::path::Path, t: std::time::SystemTime) {
        use std::os::unix::ffi::OsStrExt;
        let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as libc::time_t;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let tv = [
            libc::timeval { tv_sec: secs, tv_usec: 0 },
            libc::timeval { tv_sec: secs, tv_usec: 0 },
        ];
        unsafe {
            libc::utimes(c.as_ptr(), tv.as_ptr());
        }
    }
}
