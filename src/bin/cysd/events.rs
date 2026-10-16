//! Event bus: sequence-numbered ring buffer + broadcast push to subscribers.
//! This is the server→client push half of the bidirectional socket.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::TryLockError;
use tokio::sync::broadcast;

const RING_CAPACITY: usize = 4096;

pub struct EventBus {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Value>,
    /// seq 영속 파일 — 데몬 재시작 후에도 구독자 커서(after_seq)가 단조 증가를 유지한다.
    persist_path: Option<PathBuf>,
    /// ★(0.14.31 · 리뷰 R2) 이 세대가 물려받은 seq 바닥 = 부팅 시 읽은 **예약 상한**.
    /// 블록 예약(256)이라 그 아래 구간에는 **발행되지 않은 seq 가 섞여 있다** — 그것을 유실로
    /// 세면 재생 갭 보고가 허위가 된다. 이번 세대가 실제로 발행한 것은 전부 이 값 **초과**다.
    generation_floor: u64,
    diagnostic_publications: DiagnosticPublicationCounters,
}

struct Inner {
    seq: u64,
    ring: VecDeque<Value>,
    /// 영속된 seq 상한 — 이 값까지는 디스크에 기록 완료 (블록 예약 방식)
    persist_hwm: u64,
}

/// 이벤트 `timestamp` 의 원천 — `state::now_epoch` 과 같은 마이크로초 해상도다([`crate::state::epoch_secs_us`] · R2F-DM 2차 B1).
fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(crate::state::epoch_secs_us)
        .unwrap_or(0.0)
}

const DIAGNOSTIC_PAYLOAD_MAX_BYTES: usize = 4_096;
const DIAGNOSTIC_EVENT_MAX_BYTES: usize = 8_192;
// Two JSON numeric fields are stamped after preparation. This conservative slack
// is included in the serialized envelope bound, not claimed as heap-size accounting.
const DIAGNOSTIC_STAMP_SLACK_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiagnosticBuildError {
    PayloadTooLarge,
    InvalidJson,
    PayloadNotObject,
    EnvelopeTooLarge,
}

/// Both event copies are constructed before the event-bus lock is attempted.
/// No public fields: only the fixed identity event constructor can create this.
pub(crate) struct PreparedDiagnostic {
    ring_event: Value,
    live_event: Value,
}

impl PreparedDiagnostic {
    pub(crate) fn identity_observation(
        surface_id: Option<u64>,
        payload_json: &[u8],
    ) -> Result<Self, DiagnosticBuildError> {
        if payload_json.len() > DIAGNOSTIC_PAYLOAD_MAX_BYTES {
            return Err(DiagnosticBuildError::PayloadTooLarge);
        }
        let payload: Value = serde_json::from_slice(payload_json)
            .map_err(|_| DiagnosticBuildError::InvalidJson)?;
        if !payload.is_object() {
            return Err(DiagnosticBuildError::PayloadNotObject);
        }
        let event = json!({
            "type": "event",
            "seq": 0_u64,
            "name": "identity.observation",
            "category": "identity",
            "timestamp": 0.0_f64,
            "surface_id": surface_id,
            "payload": payload,
        });
        // Allocation and traversal are outside the lock and bounded by input size
        // plus serde_json's ordinary nesting limit. Later S10 uses typed payloads.
        let encoded = serde_json::to_vec(&event)
            .map_err(|_| DiagnosticBuildError::InvalidJson)?;
        if encoded.len().saturating_add(DIAGNOSTIC_STAMP_SLACK_BYTES)
            > DIAGNOSTIC_EVENT_MAX_BYTES
        {
            return Err(DiagnosticBuildError::EnvelopeTooLarge);
        }
        Ok(Self { ring_event: event.clone(), live_event: event })
    }

    fn stamp(&mut self, seq: u64, timestamp: f64) {
        // Shape is a private constructor invariant. Replacing existing scalar
        // values does not clone payloads or introduce arbitrary event names.
        self.ring_event["seq"] = json!(seq);
        self.live_event["seq"] = json!(seq);
        self.ring_event["timestamp"] = json!(timestamp);
        self.live_event["timestamp"] = json!(timestamp);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum DiagnosticNotPublished {
    LockBusy = 0,
    LockPoisoned = 1,
    ReservationUnavailable = 2,
    SequenceExhausted = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiagnosticPublishOutcome {
    Published { seq: u64 },
    NotPublished(DiagnosticNotPublished),
}

#[derive(Default)]
struct DiagnosticPublicationCounters {
    published: AtomicU64,
    failed: [AtomicU64; 4],
    saturated: AtomicBool,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DiagnosticFailureCounts {
    pub lock_busy: u64,
    pub lock_poisoned: u64,
    pub reservation_unavailable: u64,
    pub sequence_exhausted: u64,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DiagnosticPublicationStats {
    pub published_total: u64,
    pub not_published_total: u64,
    pub not_published_by_reason: DiagnosticFailureCounts,
    pub counter_saturated: bool,
}

impl DiagnosticPublicationCounters {
    fn increment(&self, counter: &AtomicU64) {
        // Atomic update contention is not a publication retry or a waiting mutex.
        // This is lock-free accounting, not a hard scheduler/runtime bound.
        let before = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_add(1)))
            .unwrap_or_else(|n| n);
        if before == u64::MAX {
            self.saturated.store(true, Ordering::Relaxed);
        }
    }

    fn reject(&self, reason: DiagnosticNotPublished) -> DiagnosticPublishOutcome {
        self.increment(&self.failed[reason as usize]);
        DiagnosticPublishOutcome::NotPublished(reason)
    }

    fn snapshot(&self) -> DiagnosticPublicationStats {
        let values: [u64; 4] = std::array::from_fn(|i| self.failed[i].load(Ordering::Relaxed));
        let mut aggregate_saturated = false;
        let total = values.iter().fold(0_u64, |sum, n| {
            let (next, overflow) = sum.overflowing_add(*n);
            aggregate_saturated |= overflow;
            if overflow { u64::MAX } else { next }
        });
        DiagnosticPublicationStats {
            published_total: self.published.load(Ordering::Relaxed),
            not_published_total: total,
            not_published_by_reason: DiagnosticFailureCounts {
                lock_busy: values[0],
                lock_poisoned: values[1],
                reservation_unavailable: values[2],
                sequence_exhausted: values[3],
            },
            counter_saturated: aggregate_saturated || self.saturated.load(Ordering::Relaxed),
        }
    }
}


impl EventBus {
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        let start_seq = persist_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let (tx, _) = broadcast::channel(1024);
        EventBus {
            inner: Mutex::new(Inner {
                seq: start_seq,
                ring: VecDeque::with_capacity(RING_CAPACITY),
                persist_hwm: start_seq,
            }),
            tx,
            persist_path,
            generation_floor: start_seq,
            diagnostic_publications: DiagnosticPublicationCounters::default(),
        }
    }

    /// 이 데몬 세대의 seq 바닥(그 이하는 이전 세대이거나 **미사용 예약**이다).
    pub fn generation_floor(&self) -> u64 {
        self.generation_floor
    }

    pub fn publish(&self, name: &str, category: &str, surface_id: Option<u64>, payload: Value) {
        let mut inner = self.inner.lock().unwrap();
        inner.seq += 1;
        // 블록 예약 영속: 256 seq마다 한 번만 fsync 경로를 타고, 재시작 시 예약 상한부터
        // 재개해 단조성을 유지한다 (매 publish 동기 쓰기로 전 publisher가 직렬화되던 병목 제거)
        if let Some(p) = &self.persist_path {
            if inner.seq > inner.persist_hwm {
                // 디스크에 먼저 쓰고, 성공했을 때만 메모리 hwm을 전진시킨다. write가 실패하면
                // (디스크풀·일시 I/O) hwm을 그대로 둬, 다음 publish가 같은 블록 경계에서 write를
                // 재시도한다. 메모리만 전진시키면 디스크가 영구히 뒤처져 재시작 시 이미 발행된
                // seq보다 작은 값에서 재개 → 구독자 커서 단조성이 깨진다.
                let reserved = inner.seq + 255;
                if std::fs::write(p, reserved.to_string()).is_ok() {
                    inner.persist_hwm = reserved;
                }
            }
        }
        let event = json!({
            "type": "event",
            "seq": inner.seq,
            "name": name,
            "category": category,
            "timestamp": now_epoch(),
            "surface_id": surface_id,
            "payload": payload,
        });
        if inner.ring.len() >= RING_CAPACITY {
            inner.ring.pop_front();
        }
        inner.ring.push_back(event.clone());
        // 락 보유 중 send — seq 부여 순서 = broadcast 송신 순서 보장 (역순 배달 차단).
        // broadcast::send는 비블로킹 ring buffer라 락 아래에서도 안전하다.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.tx.subscribe()
    }

    pub fn replay_after(&self, after_seq: u64) -> Vec<Value> {
        let inner = self.inner.lock().unwrap();
        inner
            .ring
            .iter()
            .filter(|e| e["seq"].as_u64().unwrap_or(0) > after_seq)
            .cloned()
            .collect()
    }

    pub fn latest_seq(&self) -> u64 {
        self.inner.lock().unwrap().seq
    }

    /// T5-2: ring의 마지막 `n`개 이벤트(시간순) — 무음 크래시 알림의 NDJSON 타임라인 tail.
    /// 바이트 상한(T4-5A)은 호출자가 응답 직렬화 경계에서 적용한다.
    pub fn tail(&self, n: usize) -> Vec<Value> {
        let inner = self.inner.lock().unwrap();
        let len = inner.ring.len();
        let start = len.saturating_sub(n);
        inner.ring.iter().skip(start).cloned().collect()
    }

    /// (ring에 남은 가장 오래된 이벤트의 seq, 현재 최신 seq) — replay 갭 감지용
    pub fn replay_bounds(&self) -> (Option<u64>, u64) {
        let inner = self.inner.lock().unwrap();
        (
            inner.ring.front().and_then(|e| e["seq"].as_u64()),
            inner.seq,
        )
    }
}

/// Does this event pass the subscriber's name/category filters?
pub fn event_matches(event: &Value, names: &[String], categories: &[String]) -> bool {
    if !names.is_empty() {
        let n = event["name"].as_str().unwrap_or("");
        if !names.iter().any(|x| x == n) {
            return false;
        }
    }
    if !categories.is_empty() {
        let c = event["category"].as_str().unwrap_or("");
        if !categories.iter().any(|x| x == c) {
            return false;
        }
    }
    true
}

impl EventBus {
    pub(crate) fn diagnostic_publication_stats(&self) -> DiagnosticPublicationStats {
        self.diagnostic_publications.snapshot()
    }

    pub(crate) fn try_publish_diagnostic(
        &self,
        mut prepared: PreparedDiagnostic,
    ) -> DiagnosticPublishOutcome {
        let mut inner = match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(TryLockError::WouldBlock) => {
                return self.diagnostic_publications.reject(DiagnosticNotPublished::LockBusy);
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                // Do not clear poison or recover/modify the legacy bus state.
                drop(poisoned);
                return self.diagnostic_publications.reject(DiagnosticNotPublished::LockPoisoned);
            }
        };
        let Some(next_seq) = inner.seq.checked_add(1) else {
            drop(inner);
            return self.diagnostic_publications.reject(DiagnosticNotPublished::SequenceExhausted);
        };
        if self.persist_path.is_some() && next_seq > inner.persist_hwm {
            drop(inner);
            return self.diagnostic_publications.reject(DiagnosticNotPublished::ReservationUnavailable);
        }

        prepared.stamp(next_seq, now_epoch());
        let evicted = if inner.ring.len() >= RING_CAPACITY {
            inner.ring.pop_front()
        } else {
            None
        };
        inner.seq = next_seq;
        inner.ring.push_back(prepared.ring_event);
        // Keep sequence assignment and broadcast order serialized as publish does.
        // Retain a no-subscriber SendError<Value> until after unlocking, too.
        // LIMIT: Tokio 1.52.3 send itself takes blocking tail/slot locks and can
        // destroy an old ordinary Value inside them. Outer try_lock is not a
        // globally nonblocking publication guarantee.
        let sent = self.tx.send(prepared.live_event);
        drop(inner);
        drop(evicted);
        drop(sent);
        self.diagnostic_publications.increment(&self.diagnostic_publications.published);
        DiagnosticPublishOutcome::Published { seq: next_seq }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str, cat: &str) -> Value {
        json!({"name": name, "category": cat})
    }
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_filters_match_all() {
        // names·categories 모두 비면 무조건 통과 (구독 전체)
        assert!(event_matches(&ev("schedule.fired", "schedule"), &[], &[]));
        assert!(event_matches(&json!({}), &[], &[]));
    }

    #[test]
    fn name_filter() {
        let e = ev("health.alert", "health");
        assert!(event_matches(&e, &v(&["health.alert"]), &[]));
        assert!(event_matches(&e, &v(&["other", "health.alert"]), &[]));
        // 불일치 name → 거부
        assert!(!event_matches(&e, &v(&["health.action"]), &[]));
    }

    #[test]
    fn category_filter() {
        let e = ev("health.alert", "health");
        assert!(event_matches(&e, &[], &v(&["health"])));
        assert!(!event_matches(&e, &[], &v(&["schedule"])));
    }

    #[test]
    fn name_and_category_are_anded() {
        let e = ev("health.alert", "health");
        // 둘 다 만족해야 통과
        assert!(event_matches(&e, &v(&["health.alert"]), &v(&["health"])));
        // name 맞고 category 틀리면 거부
        assert!(!event_matches(&e, &v(&["health.alert"]), &v(&["schedule"])));
        // category 맞고 name 틀리면 거부
        assert!(!event_matches(&e, &v(&["nope"]), &v(&["health"])));
    }

    #[test]
    fn missing_fields_use_empty_string() {
        // name 필드가 아예 없으면 ""로 취급 → 이름 필터와 불일치
        let e = json!({"category": "health"});
        assert!(!event_matches(&e, &v(&["health.alert"]), &[]));
        // category 없음 + category 필터 → 불일치
        let e2 = json!({"name": "x"});
        assert!(!event_matches(&e2, &[], &v(&["health"])));
    }

    #[test]
    fn publish_assigns_monotonic_seq_and_event_shape() {
        // seq는 publish마다 1씩 단조 증가하고, 이벤트 봉투에 모든 필드가 실린다.
        let bus = EventBus::new(None);
        assert_eq!(bus.latest_seq(), 0);
        bus.publish("a.one", "cat1", Some(7), json!({"k": 1}));
        assert_eq!(bus.latest_seq(), 1);
        bus.publish("a.two", "cat2", None, json!("p"));
        assert_eq!(bus.latest_seq(), 2);

        let all = bus.replay_after(0);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["seq"].as_u64(), Some(1));
        assert_eq!(all[0]["type"].as_str(), Some("event"));
        assert_eq!(all[0]["name"].as_str(), Some("a.one"));
        assert_eq!(all[0]["category"].as_str(), Some("cat1"));
        assert_eq!(all[0]["surface_id"].as_u64(), Some(7));
        assert_eq!(all[0]["payload"]["k"].as_u64(), Some(1));
        // None surface_id는 null로 직렬화 (필드 누락이 아님)
        assert!(all[1]["surface_id"].is_null());
        assert_eq!(all[1]["seq"].as_u64(), Some(2));
        // 단조 증가 — seq 역전 없음
        assert!(all[1]["seq"].as_u64() > all[0]["seq"].as_u64());
    }

    #[test]
    fn replay_after_is_exclusive_cursor() {
        // replay_after(n)은 seq > n 인 이벤트만 — 구독자 커서 재개의 핵심(중복·누락 없음).
        let bus = EventBus::new(None);
        for i in 0..5 {
            bus.publish(&format!("e{i}"), "c", None, json!(i));
        }
        // after_seq=0 → 전부
        assert_eq!(bus.replay_after(0).len(), 5);
        // after_seq=2 → seq 3,4,5 만 (경계 배타적: seq==2는 제외)
        let after2 = bus.replay_after(2);
        assert_eq!(after2.len(), 3);
        assert_eq!(after2[0]["seq"].as_u64(), Some(3));
        // after_seq=최신 → 빈 결과 (새 이벤트 없음)
        assert_eq!(bus.replay_after(5).len(), 0);
        // after_seq가 미래(갭)여도 패닉 없이 빈 결과
        assert_eq!(bus.replay_after(999).len(), 0);
    }

    #[test]
    fn ring_capacity_evicts_oldest_and_bounds_advance() {
        // RING_CAPACITY 초과분은 가장 오래된 것부터 퇴출 — replay_bounds의 front seq가 전진.
        let bus = EventBus::new(None);
        let over = RING_CAPACITY + 50;
        for _ in 0..over {
            bus.publish("x", "c", None, json!(null));
        }
        // 전체 seq는 over까지 증가 (단조성은 ring 퇴출과 무관)
        assert_eq!(bus.latest_seq(), over as u64);
        // ring은 RING_CAPACITY를 넘지 않는다
        let replayed = bus.replay_after(0);
        assert!(replayed.len() <= RING_CAPACITY);
        assert_eq!(replayed.len(), RING_CAPACITY);
        // 가장 오래된 보존 seq = over - RING_CAPACITY + 1 (앞 50개 퇴출)
        let (oldest, latest) = bus.replay_bounds();
        assert_eq!(latest, over as u64);
        assert_eq!(oldest, Some((over - RING_CAPACITY + 1) as u64));
        // 퇴출된 구간(after_seq < oldest)을 요청하면 갭이 드러난다:
        // 반환 길이가 (latest-after_seq)보다 작다 = 일부 이벤트 영구 손실 감지 가능
        let want = bus.replay_after(0).len() as u64;
        assert!(want < latest, "퇴출로 인해 전 구간 재생은 불가(갭 존재)");
    }

    #[test]
    fn persisted_seq_resumes_monotonic_across_restart() {
        // 영속된 hwm에서 재개 — 재시작 후에도 구독자 커서(after_seq) 단조성이 깨지지 않는다.
        let dir = std::env::temp_dir().join(format!("cys_evbus_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("seq_{}.txt", line!()));
        let _ = std::fs::remove_file(&path);

        // 1세대: 한 번 publish → hwm 블록 예약(seq+255)이 디스크에 기록됨
        {
            let bus = EventBus::new(Some(path.clone()));
            assert_eq!(bus.latest_seq(), 0);
            bus.publish("first", "c", None, json!(null));
            assert_eq!(bus.latest_seq(), 1);
        }
        let persisted: u64 = std::fs::read_to_string(&path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // 블록 예약: 첫 publish가 seq 1 + 255 = 256을 hwm으로 기록
        assert_eq!(persisted, 256);

        // 2세대: 재시작 — start_seq가 영속 hwm에서 재개되어 절대 역전하지 않는다
        {
            let bus2 = EventBus::new(Some(path.clone()));
            assert_eq!(bus2.latest_seq(), persisted);
            bus2.publish("after-restart", "c", None, json!(null));
            // 새 이벤트 seq는 이전 세대 최대 seq(1)보다 반드시 크다 — 커서 충돌 없음
            assert_eq!(bus2.latest_seq(), persisted + 1);
            assert!(bus2.latest_seq() > 1);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persist_write_failure_does_not_break_resume_monotonicity() {
        // 회귀 박제: 블록 예약 영속 write가 실패(디스크풀·일시 I/O)해도 in-memory hwm을
        // 멋대로 전진시키면 안 된다. 전진하면 다음 publish들이 같은 블록 안이라 write를
        // 재시도하지 않아 디스크가 영구히 뒤처지고, 재시작 시 이미 발행된 seq보다 작은
        // 값에서 재개 → 구독자 커서(after_seq) 단조성 붕괴.
        //
        // 결정론적 실패 주입: persist 경로를 '디렉터리'로 만들면 std::fs::write가 항상
        // 실패한다(디스크풀/일시 I/O의 대역). 블록 경계(seq 257)에서만 실패시키고 그 직후
        // 파일로 복구해, 다음 publish의 write 재시도가 디스크를 따라잡는지로 fix를 판별한다.
        let dir = std::env::temp_dir().join(format!(
            "cys_evbus_wfail_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("event.seq");

        // 1세대: 첫 블록(seq 1 → hwm 256) write 성공 → 디스크=256.
        {
            let bus = EventBus::new(Some(path.clone()));
            bus.publish("g1", "c", None, json!(null));
            assert_eq!(bus.latest_seq(), 1);
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim().parse::<u64>().unwrap(),
            256
        );

        // 2세대: 디스크=256에서 재개. start_seq=hwm=256이라 *첫* publish(seq 257)가 곧
        // 블록 경계 write를 트리거한다. 이 write만 실패시키려고 경로를 디렉터리로 점유했다가
        // 직후 파일로 복구해, 다음 publish(seq 258)의 write 재시도가 디스크를 따라잡는지 본다.
        let bus = EventBus::new(Some(path.clone()));
        assert_eq!(bus.latest_seq(), 256);

        // seq 257: 블록 경계 → write 시도. 경로를 디렉터리로 만들어 *이 write만* 실패.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        bus.publish("boundary", "c", None, json!(null));
        assert_eq!(bus.latest_seq(), 257);
        // 디렉터리였으니 257의 write는 실패 — 디스크엔 새 정수가 없다.
        std::fs::remove_dir_all(&path).unwrap();

        // seq 258..400: 경로 복구됨(파일 write 가능). 발행을 계속한다.
        for _ in 0..143 {
            bus.publish("g2b", "c", None, json!(null));
        }
        let highest_emitted = bus.latest_seq();
        assert_eq!(highest_emitted, 400);
        drop(bus);

        // 재시작 시뮬레이션: 디스크 영속값에서 재개.
        let resumed = EventBus::new(Some(path.clone()));
        // 버그: 257 write 실패에도 hwm이 512로 전진 → 258..400 발행이 같은 블록(<=512)이라
        //       write 재시도 안 함 → 디스크는 256에 갇힘 → resume=256 < 400 = 단조성 붕괴.
        // 수정: 257 write 실패 시 hwm은 256 유지 → 258 발행이 write 재시도 → 디스크가
        //       258+255=513으로 따라잡힘 → resume=513 >= 400 = 단조성 보존.
        assert!(
            resumed.latest_seq() >= highest_emitted,
            "재시작 seq {}이 이미 발행된 최대 seq {}보다 작다 — 단조성 붕괴",
            resumed.latest_seq(),
            highest_emitted
        );

        let _ = std::fs::remove_dir_all(&dir);
    }


    fn prepared_identity() -> PreparedDiagnostic {
        PreparedDiagnostic::identity_observation(
            Some(7), br#"{"reason":"unproven","request_sequence":9}"#,
        ).expect("bounded fixture")
    }

    fn diagnostic_seq_path() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cys_diagnostic_api_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("event.seq")
    }

    #[test]
    fn diagnostic_input_is_bounded_and_identity_only() {
        assert!(matches!(
            PreparedDiagnostic::identity_observation(None, &vec![b' '; DIAGNOSTIC_PAYLOAD_MAX_BYTES + 1]),
            Err(DiagnosticBuildError::PayloadTooLarge)
        ));
        assert!(matches!(
            PreparedDiagnostic::identity_observation(None, b"[]"),
            Err(DiagnosticBuildError::PayloadNotObject)
        ));
        assert!(matches!(
            PreparedDiagnostic::identity_observation(None, b"{"),
            Err(DiagnosticBuildError::InvalidJson)
        ));
        let p = prepared_identity();
        assert_eq!(p.ring_event, p.live_event);
        assert_eq!(p.ring_event["name"], "identity.observation");
        assert_eq!(p.ring_event["category"], "identity");
        assert!(serde_json::to_vec(&p.ring_event).unwrap().len() + DIAGNOSTIC_STAMP_SLACK_BYTES
            <= DIAGNOSTIC_EVENT_MAX_BYTES);
    }

    #[test]
    fn diagnostic_cold_boundary_and_restart_refuse_without_seq_or_file_change() {
        let path = diagnostic_seq_path();
        let bus = EventBus::new(Some(path.clone()));
        let rejected = DiagnosticPublishOutcome::NotPublished(DiagnosticNotPublished::ReservationUnavailable);
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()), rejected);
        assert_eq!(bus.latest_seq(), 0);
        assert!(bus.replay_after(0).is_empty());
        assert!(!path.exists(), "diagnostic created a sequence reservation file");
        assert_eq!(bus.diagnostic_publication_stats().not_published_by_reason.reservation_unavailable, 1);

        // Existing ordinary API is the only reservation writer; no diagnostic fallback.
        bus.publish("test.operational", "system", None, json!(null));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, b"256");
        for expected in 2..=256 {
            assert_eq!(bus.try_publish_diagnostic(prepared_identity()),
                DiagnosticPublishOutcome::Published { seq: expected });
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let before = bus.replay_after(0);
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()), rejected);
        assert_eq!(bus.latest_seq(), 256);
        assert_eq!(bus.replay_after(0), before);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(bus.diagnostic_publication_stats().published_total, 255);
        assert_eq!(bus.diagnostic_publication_stats().not_published_total, 2);

        let restarted = EventBus::new(Some(path.clone()));
        assert_eq!(restarted.generation_floor(), 256);
        assert_eq!(restarted.try_publish_diagnostic(prepared_identity()), rejected);
        assert_eq!(restarted.latest_seq(), 256);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // An actual ordinary publish can restore capacity, without modifying that API.
        restarted.publish("test.operational", "system", None, json!(null));
        assert_eq!(restarted.try_publish_diagnostic(prepared_identity()),
            DiagnosticPublishOutcome::Published { seq: 258 });
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn diagnostic_busy_returns_before_held_lock_is_released() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;
        let bus = Arc::new(EventBus::new(None));
        let held = bus.inner.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let other = Arc::clone(&bus);
        let worker = std::thread::spawn(move || {
            let result = other.try_publish_diagnostic(prepared_identity());
            tx.send(result).unwrap();
        });
        // Generous synchronization bound, not a product performance-budget assertion.
        let before_release = rx.recv_timeout(Duration::from_secs(5));
        // Release before any assert/join: a blocking-lock mutant must fail cleanly,
        // not leave a permanently blocked test thread.
        drop(held);
        worker.join().unwrap();
        assert_eq!(before_release.expect("diagnostic waited for the held bus lock"),
            DiagnosticPublishOutcome::NotPublished(DiagnosticNotPublished::LockBusy));
        assert_eq!(bus.latest_seq(), 0);
        assert!(bus.replay_after(0).is_empty());
        assert_eq!(bus.diagnostic_publication_stats().not_published_by_reason.lock_busy, 1);
        // Subsequent ordinary publication progresses normally after external contention ends.
        bus.publish("test.operational", "system", None, json!(null));
        assert_eq!(bus.latest_seq(), 1);
    }

    #[test]
    fn diagnostic_poison_is_counted_without_recovery_or_publication() {
        let bus = EventBus::new(None);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = bus.inner.lock().unwrap();
            panic!("fixture poison");
        }));
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()),
            DiagnosticPublishOutcome::NotPublished(DiagnosticNotPublished::LockPoisoned));
        assert!(bus.inner.is_poisoned(), "diagnostic silently repaired legacy poison state");
        assert_eq!(bus.diagnostic_publication_stats().not_published_by_reason.lock_poisoned, 1);
        let state = match bus.inner.lock() {
            Err(poisoned) => poisoned.into_inner(),
            Ok(_) => panic!("expected poisoned bus"),
        };
        assert_eq!(state.seq, 0);
        assert!(state.ring.is_empty());
    }

    #[test]
    fn diagnostic_success_is_one_ring_and_broadcast_event_with_matching_seq() {
        let bus = EventBus::new(None);
        let mut rx = bus.subscribe();
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()),
            DiagnosticPublishOutcome::Published { seq: 1 });
        let ring = bus.replay_after(0);
        assert_eq!(ring.len(), 1);
        assert_eq!(rx.try_recv().unwrap(), ring[0]);
        assert!(matches!(rx.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
        assert_eq!(ring[0]["seq"], 1);
        assert_eq!(ring[0]["surface_id"], 7);
        assert_eq!(ring[0]["payload"]["request_sequence"], 9);
        let stats = bus.diagnostic_publication_stats();
        assert_eq!(stats.published_total, 1);
        assert_eq!(stats.not_published_total, 0);
    }

    #[test]
    fn diagnostic_without_subscriber_is_still_published_to_replay_ring() {
        let bus = EventBus::new(None);
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()),
            DiagnosticPublishOutcome::Published { seq: 1 });
        assert_eq!(bus.replay_after(0).len(), 1);
        assert_eq!(bus.diagnostic_publication_stats().not_published_total, 0);
    }

    #[test]
    fn diagnostic_sequence_exhaustion_is_not_saturation_or_wraparound() {
        let bus = EventBus::new(None);
        {
            let mut state = bus.inner.lock().unwrap();
            state.seq = u64::MAX;
            state.persist_hwm = u64::MAX;
        }
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()),
            DiagnosticPublishOutcome::NotPublished(DiagnosticNotPublished::SequenceExhausted));
        assert_eq!(bus.latest_seq(), u64::MAX);
        assert!(bus.replay_after(0).is_empty());
        assert_eq!(bus.diagnostic_publication_stats().not_published_by_reason.sequence_exhausted, 1);
    }

    #[test]
    fn diagnostic_reason_counters_saturate_and_disclose_loss() {
        let bus = EventBus::new(None);
        bus.diagnostic_publications.failed[DiagnosticNotPublished::LockBusy as usize]
            .store(u64::MAX, Ordering::Relaxed);
        let held = bus.inner.lock().unwrap();
        assert_eq!(bus.try_publish_diagnostic(prepared_identity()),
            DiagnosticPublishOutcome::NotPublished(DiagnosticNotPublished::LockBusy));
        // Stats accessor does not acquire the bus lock.
        let stats = bus.diagnostic_publication_stats();
        drop(held);
        assert_eq!(stats.not_published_total, u64::MAX);
        assert_eq!(stats.not_published_by_reason.lock_busy, u64::MAX);
        assert!(stats.counter_saturated);
    }

}
