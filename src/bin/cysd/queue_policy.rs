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

    // 신설 3종 (C2·C3·C4)
    pub const REJECTED: &str = "queue.rejected";
    pub const EXPIRED: &str = "queue.expired";
    pub const MERGED: &str = "queue.merged";
}

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
    /// 기본 — 이벤트만 남기고 적재는 허용한다(관찰 기간). 차단이 유예됐을 뿐
    /// TTL(C3)+OOB(C5)는 활성이므로 적체 상한은 TTL 시간으로 묶인다.
    Log,
    /// 결정론 거부 — 발신자에게 오류 + 데몬이 dead-letter에 전문 기록.
    Enforce,
}

pub fn policy_mode() -> PolicyMode {
    match std::env::var("CYS_QUEUE_POLICY_MODE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "enforce" => PolicyMode::Enforce,
        _ => PolicyMode::Log,
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

/// 원장 사유. **어떤 경로로도 무음 삭제는 없다** — 큐에서 사라지는 모든 항목은 여기에 남는다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadLetterReason {
    Ttl,
    SoftcapRejected,
    Superseded,
    WakeupCap,
    ClearedByOperator,
}

impl DeadLetterReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeadLetterReason::Ttl => "ttl",
            DeadLetterReason::SoftcapRejected => "softcap_rejected",
            DeadLetterReason::Superseded => "superseded",
            DeadLetterReason::WakeupCap => "wakeup_cap",
            DeadLetterReason::ClearedByOperator => "cleared_by_operator",
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
                       "hint": "관찰 모드 — 적재는 허용됐다. 차단은 CYS_QUEUE_POLICY_MODE=enforce"}),
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

/// 발신 origin 판별(D2). **자기신고 `from`은 쓰지 않는다** — 커널 peer pid로 해석된
/// `verified_from`만이 근거다.
/// 우선순위: verified & agent_meta 보유 → Agent · human 신고 → Human · 그 외 → System.
pub fn derive_origin(daemon: &Daemon, verified_from: Option<u64>, human: bool) -> QueueOrigin {
    if let Some(sid) = verified_from {
        if let Some(s) = daemon.get_surface(sid) {
            if s.agent_meta.lock().unwrap().is_some() {
                return QueueOrigin::Agent {
                    surface: sid,
                    role: s.role.lock().unwrap().clone(),
                };
            }
        }
        // verified 이지만 에이전트 미등록(맨 셸 pane) — 정책 대상 아님.
        return if human {
            QueueOrigin::Human
        } else {
            QueueOrigin::system("pane")
        };
    }
    if human {
        QueueOrigin::Human
    } else {
        // 조상추적 미해소(외부 프로세스·데몬 내부) — 위협 모델상 정책 밖.
        QueueOrigin::system("anonymous")
    }
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

    /// T2-b: 모드 파싱 — 미설정·오타·대소문자. 기본은 관찰(log)이며 오타로 enforce가 되면 안 된다.
    #[test]
    fn policy_mode_defaults_to_log() {
        let cases = [("", PolicyMode::Log), ("log", PolicyMode::Log),
                     ("garbage", PolicyMode::Log), ("ENFORCE", PolicyMode::Enforce),
                     (" enforce ", PolicyMode::Enforce)];
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
    }
}
