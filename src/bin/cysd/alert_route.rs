//! ★(0.14.31 · WP-3 B) 데몬 alert → CSO inbox 라우팅.
//!
//! 【무엇을 고치는가】 CSO 지침은 "상시 `cys events` 를 구독하라" 였다. 그 구독은 좌석의 컨텍스트를
//! 상시 태우고(치명위험 ②·③), 구독이 끊기면 경보가 **아무데도 도착하지 않는다**. 데몬이 이미 보고
//! 있는 사실을 데몬이 **한 줄로 밀어 넣는** 것이 옳다 — 규약이 아니라 장치(§3-1).
//!
//! 【구조】 세 층으로 갈랐다.
//!   ① **순수 판정**([`decide`]·[`summarize`]·[`render_text`]) — 데몬 상태를 인자로만 받는다.
//!      폭풍 억제의 모든 규칙이 여기 있고, 단위 검체가 이 층만으로 봉인을 증명한다(PTY 불요 =
//!      Windows 에서도 도는 검체).
//!   ② **적재**([`enqueue_alert`]) — 기존 enqueue 3경로와 **동형**이다(`next_queue_entry` →
//!      `pending_queue` → `queue.enqueued` → `persist_queue_state`). 배달 규칙·WAL·원장·좌석
//!      게이트·pause 는 **하나도 건드리지 않는다**(§8: 새 신호가 기존 게이트를 면제하지 않는다).
//!   ③ **구독 태스크**([`spawn`]) — `boot_supervisor::spawn` 규약 동형의 **별도 tokio 태스크**다.
//!      `EventBus::publish` 안에서 도는 동기 콜백은 금지다(publish 는 inner 락을 쥔 채 broadcast
//!      한다 — 그 안에서 큐 락을 잡으면 전 publisher 가 직렬화되고 락 역순이 생긴다).
//!
//! 【폭주 봉인(§7 ①)】 (name,surface) 5분 쿨다운 · 시간당 20건 상한 · CSO **역할 좌석 전체**의
//! 자기 이벤트 제외 · 데몬 부트 300초 유예 · CSO 활성 큐 보호선(50). 억제된 것은 **버리지 않고**
//! [`RouteState::pending`] 에 키 단위로 병합해 두었다가 유예 종료·CSO 착석·쿨다운 만료 시
//! 재평가해 **키당 정확히 1건**으로 적재한다. `context.threshold` 는 에지 1회 발행이라 버리면
//! 영영 오지 않는다(치명위험 ②의 직접 경로).
//!
//! 【단일 실행자】 이 모듈의 상태는 `Daemon::alert_route` 한 벌이고, 판정→적재→기록을 **한 태스크가
//! 순차로** 돈다. 두 실행자가 동시에 돌면 19건 상태에서 둘 다 Route 를 승인해 상한이 깨진다 —
//! [`spawn`] 은 재호출을 스스로 거절한다(멱등).

use crate::state::{now_epoch, Daemon};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 롤백 스위치(§3-4: 게이트를 끄는 노브가 아니라 **신 기능**의 롤백이다 · 기본 켬).
/// `0`·`false`·`off`·`no` → 구독 태스크를 열지 않는다(`enabled=false` 로 status 에 정직히 보인다).
pub const ENV_ALERT_ROUTE: &str = "CYS_ALERT_ROUTE";

/// (name,surface) 쿨다운(초) — 같은 사실을 5분 안에 두 번 밀지 않는다.
pub const COOLDOWN_SECS: f64 = 300.0;
/// 시간당 적재 상한(건) — 전 키 합산. 넘으면 **보류**(폐기 아님).
pub const HOURLY_CAP: usize = 20;
/// 상한 창(초).
pub const WINDOW_SECS: f64 = 3600.0;
/// 데몬 부트 유예(초) — 기동 폭풍(복원·재조정 이벤트)이 CSO 좌석을 덮지 않게.
pub const BOOT_GRACE_SECS: f64 = 300.0;
/// 미해결 집합의 키 상한. 넘으면 **가장 오래된 키를 버리지 않고** 요약 키
/// ([`OVERFLOW_NAME`])로 접는다 — 폐기 0 계약을 유지하면서 24/365 메모리를 유계로 만든다.
pub const PENDING_MAX: usize = 512;
/// 접힌 미해결분의 요약 키 이름. **이 문자열로 버스 이벤트를 발행하지 않는다** —
/// 발행하면 자기 이벤트를 다시 라우팅하는 되먹임이 생긴다(회귀 핀이 이 사실을 박제한다).
pub const OVERFLOW_NAME: &str = "alert_route.overflow";
/// CSO 활성 큐 보호선(항목) — 경보가 이 깊이 이상을 차지하지 않는다. 활성 큐 상한(100)의 절반을
/// 사람·노드의 실제 보고 몫으로 남긴다(경보가 업무 메시지를 밀어내면 그것이 곧 폭주다).
pub const CSO_QUEUE_HEADROOM: usize = 50;
/// 재평가 틱 주기(초).
pub const REEVAL_INTERVAL_SECS: u64 = 30;
/// 한 재평가 틱이 **훑는** 최대 키 수. 적재 건수 상한이 아니다(상한은 [`HOURLY_CAP`] 이 진다) —
/// 둘을 같은 숫자로 묶으면 쿨다운 중인 앞쪽 키들이 뒤쪽의 적재 가능한 키를 굶긴다.
pub const REEVAL_SCAN_MAX: usize = 256;

/// 적재 경로 태그(`QueueEntry::origin`) — 계약 문자열(CONTRACTS §C).
pub const ALERT_ORIGIN: &str = "alert";
/// 발신자 라벨(`QueueEntry::from`) — surface ref 가 아니므로 원장에서는 `from_label` 로 간다
/// (§8 "`from` 에 임의 문자열을 넣지 않는다" = surface ref 계약 준수).
pub const ALERT_FROM: &str = "daemon";

/// 요약 1줄의 바이트 상한(문자 경계 절단).
const SUMMARY_MAX_BYTES: usize = 200;

/// 라우팅 대상 이벤트인가(정본 §4 WP-3 B 목록 그대로 + 내부 요약 키).
pub fn routable(name: &str) -> bool {
    name == OVERFLOW_NAME
        || matches!(
            name,
            "health.alert"
                | "surface.exited"
                | "context.threshold"
                | "queue.starved"
                | "queue.depth_high"
        )
        || name.starts_with("watchdog.")
}

/// 억제·상한의 키. **surface 는 `Option`** 이다 — 좌석 없는 경보(`watchdog.load_high` 등)가
/// 존재하기 때문이고, 그래서 자기제외 비교는 반드시 "실재하는 좌석 id 와의 일치" 여야 한다
/// ([`decide`] ① · `None == None` 이 true 인 것이 이 자료형의 함정이다).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AlertKey {
    pub name: String,
    pub surface: Option<u64>,
}

impl AlertKey {
    pub fn new(name: &str, surface: Option<u64>) -> Self {
        AlertKey { name: name.to_string(), surface }
    }
}

/// 이벤트 1건에서 뽑은 라우팅 재료.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertItem {
    pub key: AlertKey,
    /// 1줄 요약(제어문자 제거·공백 압축·바이트 상한).
    pub summary: String,
}

/// 미해결 항목 — **키 단위로 병합**한다. 폭풍 100건이 100줄이 되면 그것이 폭주다.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingAlert {
    pub first_seen: f64,
    pub last_seen: f64,
    /// 이 키로 관측된 횟수(병합분 포함) — 적재 문안에 `(반복 N건)` 으로 실린다.
    pub count: u64,
    pub summary: String,
    /// 마지막 보류 사유(관측용).
    pub reason: &'static str,
}

/// 보류 사유. 전부 **되돌아올 수 있는** 상태다(그래서 폐기가 아니라 보류다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldReason {
    BootGrace,
    Paused,
    NoCso,
    Cooldown,
    HourlyCap,
    QueueHeadroom,
}

impl HoldReason {
    pub fn as_str(self) -> &'static str {
        match self {
            HoldReason::BootGrace => "boot_grace",
            HoldReason::Paused => "paused",
            HoldReason::NoCso => "no_cso",
            HoldReason::Cooldown => "cooldown",
            HoldReason::HourlyCap => "hourly_cap",
            HoldReason::QueueHeadroom => "queue_headroom",
        }
    }

    /// 이 사유가 **키와 무관한 전역 상태**인가(유예·동결·부재·상한·큐 보호선).
    /// 전역이면 재평가 순회를 계속할 이유가 없다(뒤 키도 같은 답을 받는다).
    pub fn is_global(self) -> bool {
        !matches!(self, HoldReason::Cooldown)
    }
}

/// 라우팅 대상이 **아닌** 사유 — 보류하지 않는다(되돌아올 상태가 아니다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreReason {
    NotRoutable,
    CsoOwnSurface,
}

impl IgnoreReason {
    pub fn as_str(self) -> &'static str {
        match self {
            IgnoreReason::NotRoutable => "not_routable",
            IgnoreReason::CsoOwnSurface => "cso_own_surface",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Route,
    Hold(HoldReason),
    Ignore(IgnoreReason),
}

/// 판정에 필요한 **데몬 상태의 사본**. 순수 판정이 데몬을 직접 읽지 않게 하는 경계다.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteCtx {
    pub now: f64,
    pub daemon_started_at: f64,
    /// 적재 대상 CSO 좌석(없으면 None — 부재는 보류 사유이지 폐기 사유가 아니다).
    pub cso_surface: Option<u64>,
    /// **살아있는 CSO 역할 좌석 전체**. 자기제외는 이 집합으로 한다 — `cso`·`cso-2` 처럼
    /// 좌석이 둘이면 "A 의 적체를 B 에게, B 의 적체를 A 에게" 미는 순환이 생긴다.
    pub cso_seats: Vec<u64>,
    /// 배달 동결(kill-switch `daemon.paused` ∨ CSO 좌석 `queue_paused_until` 미래).
    pub delivery_frozen: bool,
    /// CSO 활성 큐 현재 깊이(보호선 판정용).
    pub cso_queue_depth: usize,
}

/// 분(minute) 버킷 창 — 폭풍 입력만큼 커지는 타임스탬프 배열을 쓰지 않기 위한 유계 계수기.
/// 최대 61개 버킷(1시간+현재분)이라 입력이 초당 수천 건이어도 메모리가 자라지 않는다.
#[derive(Debug, Default)]
pub struct MinuteWindow {
    buckets: VecDeque<(i64, u64)>,
}

impl MinuteWindow {
    fn bucket_of(now: f64) -> i64 {
        (now / 60.0).floor() as i64
    }

    pub fn add(&mut self, now: f64) {
        let b = Self::bucket_of(now);
        self.prune(now);
        match self.buckets.back_mut() {
            Some((mb, c)) if *mb == b => *c += 1,
            _ => self.buckets.push_back((b, 1)),
        }
    }

    fn prune(&mut self, now: f64) {
        let cutoff = Self::bucket_of(now) - 60;
        while self.buckets.front().is_some_and(|(b, _)| *b < cutoff) {
            self.buckets.pop_front();
        }
    }

    pub fn count(&self, now: f64) -> usize {
        let cutoff = Self::bucket_of(now) - 60;
        self.buckets
            .iter()
            .filter(|(b, _)| *b >= cutoff)
            .map(|(_, c)| *c as usize)
            .sum()
    }
}

/// 라우터의 살아있는 상태. **전용 Mutex 하나**에 담긴다 — 큐 계열 락 순서 규약
/// (restored_queue → surfaces → pending_queue) **밖**의 독립 락이고, 이 락을 쥔 채
/// 다른 어떤 락도 잡지 않는다(그래서 락쌍이 만들어지지 않는다).
#[derive(Debug, Default)]
pub struct RouteState {
    /// 구독 태스크가 실제로 떴는가(env 롤백이면 false — status 에 정직히 보인다).
    pub enabled: bool,
    /// (name,surface) → 마지막 적재 시각.
    pub last_routed: HashMap<AlertKey, f64>,
    /// (name,surface) → 마지막 **제외 관측 발행** 시각. 자기 좌석 이벤트는 헬스 룰 디바운스(30s)만
    /// 타고 계속 온다 — 그때마다 `alert_route.ignored` 를 발행하면 관측이 그 자체로 버스 소음이 된다
    /// (룰 10종이면 시간당 1,200줄). 발행에도 같은 쿨다운을 걸어 "억제도 보이되 소음이 되지는 않게"
    /// 한다. **판정에는 쓰이지 않는다**(제외는 시간이 지나도 제외다).
    pub last_ignored: HashMap<AlertKey, f64>,
    /// 최근 적재 시각들(상한 창) — 상한(20/h) 자체가 길이를 유계로 만든다.
    pub routed_window: VecDeque<f64>,
    /// 최근 보류 계수(분 버킷 — 입력 폭풍에 메모리가 자라지 않는다).
    pub suppressed_window: MinuteWindow,
    /// 미해결 집합 — 폐기 0의 보관처.
    pub pending: BTreeMap<AlertKey, PendingAlert>,
    pub routed_total: u64,
    pub suppressed_total: u64,
    /// 상한 초과로 요약 키에 **접힌** 미해결 키 수(폐기가 아니라 접기 — 침묵 금지 카운터).
    pub folded_total: u64,
}

impl RouteState {
    /// 창 밖 항목 정리 — 24/365 데몬의 무한 성장 차단.
    fn prune(&mut self, now: f64) {
        while self.routed_window.front().is_some_and(|t| now - *t > WINDOW_SECS) {
            self.routed_window.pop_front();
        }
        // 쿨다운 맵도 창(쿨다운의 2배)을 넘긴 항목은 어떤 판정에도 쓰이지 않는다
        // (죽은 좌석의 키가 데몬 수명 내내 남지 않게).
        self.last_routed.retain(|_, t| now - *t <= 2.0 * COOLDOWN_SECS);
        self.last_ignored.retain(|_, t| now - *t <= 2.0 * COOLDOWN_SECS);
    }

    /// 제외 관측을 지금 발행해도 되는가(쿨다운 1개 창) — 발행하기로 하면 시각을 세운다.
    pub fn should_publish_ignored(&mut self, key: &AlertKey, now: f64) -> bool {
        self.prune(now);
        if self.last_ignored.get(key).is_some_and(|t| now - *t < COOLDOWN_SECS) {
            return false;
        }
        self.last_ignored.insert(key.clone(), now);
        true
    }

    pub fn routed_1h(&self, now: f64) -> usize {
        self.routed_window.iter().filter(|t| now - **t <= WINDOW_SECS).count()
    }

    pub fn suppressed_1h(&self, now: f64) -> usize {
        self.suppressed_window.count(now)
    }

    /// 적재 성공 기록.
    pub fn record_routed(&mut self, key: &AlertKey, now: f64) {
        self.prune(now);
        self.last_routed.insert(key.clone(), now);
        self.routed_window.push_back(now);
        self.routed_total += 1;
        self.pending.remove(key);
    }

    /// 보류 기록 — 키 단위 병합. 반환값은 상한 초과로 **접힌 키**(있으면 이벤트로 남긴다).
    pub fn record_hold(
        &mut self,
        key: &AlertKey,
        summary: &str,
        reason: HoldReason,
        now: f64,
    ) -> Option<AlertKey> {
        self.prune(now);
        self.suppressed_window.add(now);
        self.suppressed_total += 1;
        match self.pending.get_mut(key) {
            Some(p) => {
                p.last_seen = now;
                p.count += 1;
                p.summary = summary.to_string();
                p.reason = reason.as_str();
                None
            }
            None => {
                self.pending.insert(
                    key.clone(),
                    PendingAlert {
                        first_seen: now,
                        last_seen: now,
                        count: 1,
                        summary: summary.to_string(),
                        reason: reason.as_str(),
                    },
                );
                self.fold_overflow(key, now)
            }
        }
    }

    /// 미해결 집합이 상한을 넘으면 **가장 오래된** 키를 요약 키([`OVERFLOW_NAME`])로 접는다.
    ///
    /// 【왜 폐기하지 않는가】 `context.threshold` 는 임계 위 체류 동안 재발행되지 않는다
    /// (handlers `maybe_fire_context_threshold` 의 에지 래치) — 그 한 건을 버리면 CSO 는
    /// 컨텍스트 60% 초과를 **영영** 모른다(치명위험 ②). 그래서 상한은 "버림" 이 아니라
    /// "한 줄로 접음" 이다. 접힌 뒤 남는 정직한 한계: 어느 좌석의 어떤 이름이었는지는 사라지고
    /// "미해결 N종이 상한을 넘었다" 는 사실만 CSO 에게 간다(그 사실 자체가 조치 신호다).
    fn fold_overflow(&mut self, just_added: &AlertKey, now: f64) -> Option<AlertKey> {
        if self.pending.len() <= PENDING_MAX {
            return None;
        }
        let overflow_key = AlertKey::new(OVERFLOW_NAME, None);
        let victim = self
            .pending
            .iter()
            .filter(|(k, _)| *k != just_added && k.name != OVERFLOW_NAME)
            .min_by(|a, b| {
                a.1.first_seen
                    .partial_cmp(&b.1.first_seen)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            })
            .map(|(k, _)| k.clone())?;
        let folded = self.pending.remove(&victim)?;
        self.folded_total += 1;
        let entry = self.pending.entry(overflow_key).or_insert(PendingAlert {
            first_seen: folded.first_seen,
            last_seen: now,
            count: 0,
            summary: String::new(),
            reason: "folded",
        });
        entry.count += folded.count.max(1);
        entry.last_seen = now;
        entry.first_seen = entry.first_seen.min(folded.first_seen);
        entry.summary = format!(
            "미해결 경보가 상한({PENDING_MAX}종)을 넘어 요약으로 접혔다 — 접힌 종류 {} · cys queue list 와 cys events 로 원본을 확인하라",
            self.folded_total
        );
        Some(victim)
    }

    /// `cys status --json` 의 `alert_route` — **정확히 이 4키**(CONTRACTS §C).
    pub fn snapshot(&self, now: f64) -> Value {
        json!({
            "enabled": self.enabled,
            "routed_1h": self.routed_1h(now),
            "suppressed_1h": self.suppressed_1h(now),
            "pending": self.pending.len(),
        })
    }
}

/// ★순수 판정. 순서가 계약이다.
///
/// ① 자기 이벤트 제외 → ② 부트 유예 → ③ 배달 동결(pause) → ④ CSO 부재 → ⑤ 쿨다운 →
/// ⑥ 시간당 상한 → ⑦ CSO 큐 보호선 → Route.
///
/// **①만 `Ignore`(폐기)이고 ②~⑦은 전부 `Hold`(보류)** 다: 자기 좌석 이벤트는 시간이 지나도
/// 라우팅 대상이 되지 않지만, 나머지는 전부 되돌아올 수 있는 상태다.
pub fn decide(state: &RouteState, key: &AlertKey, ctx: &RouteCtx) -> Verdict {
    if !routable(&key.name) {
        return Verdict::Ignore(IgnoreReason::NotRoutable);
    }
    // ★★ ① 자기제외는 **실재하는 CSO 좌석 id 와 일치할 때만** 성립한다. `Option<u64>` 두 개를
    //    `==` 로 비교하면 "좌석 없는 경보(None)" 와 "CSO 부재(None)" 가 같다고 판정돼 — 정확히
    //    CSO 가 없어서 보관해야 할 그 순간에 — 전 경보가 '자기 이벤트'로 조용히 폐기된다
    //    (결측은 값이 아니다). 집합 비교는 그 함정을 구조적으로 없앤다.
    if key.surface.is_some_and(|s| ctx.cso_seats.contains(&s)) {
        return Verdict::Ignore(IgnoreReason::CsoOwnSurface);
    }
    if ctx.now - ctx.daemon_started_at < BOOT_GRACE_SECS {
        return Verdict::Hold(HoldReason::BootGrace);
    }
    if ctx.delivery_frozen {
        return Verdict::Hold(HoldReason::Paused);
    }
    if ctx.cso_surface.is_none() {
        return Verdict::Hold(HoldReason::NoCso);
    }
    if let Some(last) = state.last_routed.get(key) {
        if ctx.now - *last < COOLDOWN_SECS {
            return Verdict::Hold(HoldReason::Cooldown);
        }
    }
    if state.routed_1h(ctx.now) >= HOURLY_CAP {
        return Verdict::Hold(HoldReason::HourlyCap);
    }
    if ctx.cso_queue_depth >= CSO_QUEUE_HEADROOM {
        return Verdict::Hold(HoldReason::QueueHeadroom);
    }
    Verdict::Route
}

/// 제어문자 제거 · 공백 압축 · 문자경계 바이트 절단. 화면 원문이 그대로 pane 에 들어가는 것을
/// 막는 유일한 층이다(경보 본문은 **사실의 요약**이지 화면 사본이 아니다).
pub fn sanitize_line(s: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_bytes));
    let mut prev_space = false;
    for ch in s.chars() {
        // ★투명문자(Cf: ZWSP·ZWNJ·BOM·방향지시자 등)는 **공백으로 바꾸지 않고 지운다**.
        //   `char::is_control()` 은 Cc 만 잡아 이것들을 통과시킨다 — 눈에 안 보이는 글자가 pane
        //   문안에 남으면 판독자(`schedule::has_machine_label` 등 같은 Cf 목록을 쓰는 층)가
        //   라벨·문면을 다르게 읽는다. 목록은 그 함수와 같은 실사용 집합이다.
        if matches!(ch, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}'
            | '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{2066}'..='\u{2069}'
            | '\u{202a}'..='\u{202e}')
        {
            continue;
        }
        let c = if ch.is_control() { ' ' } else { ch };
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
            continue;
        }
        prev_space = false;
        out.push(c);
    }
    let trimmed = out.trim_end();
    if trimmed.len() <= max_bytes {
        return trimmed.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    trimmed[..cut].trim_end().to_string()
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn field(payload: &Value, k: &str) -> Option<String> {
    payload.get(k).and_then(scalar)
}

/// 요약에 **싣지 않는** 키: 화면 원문·조치 안내·자유 문장. 이것들이 pane 문안으로 들어가면
/// LLM 이 그 문장을 지시로 읽는다(경보는 사실만 전한다 · `queue.starved` hint 계약과 같은 이유).
const SUMMARY_DENY_KEYS: &[&str] = &["line", "hint", "note", "action", "text", "message", "preview"];

/// 이벤트 payload → 1줄 요약. 알려진 이벤트는 고정 서식, 그 밖(watchdog.*)은 정렬된 스칼라 4개.
pub fn summarize_payload(name: &str, payload: &Value) -> String {
    let s = match name {
        "health.alert" => field(payload, "rule").map(|r| format!("rule={r}")),
        "surface.exited" => {
            let role = field(payload, "role").unwrap_or_else(|| "-".into());
            let agent = field(payload, "agent").unwrap_or_else(|| "-".into());
            Some(format!("role={role} agent={agent}"))
        }
        "context.threshold" => {
            let role = field(payload, "role").unwrap_or_else(|| "-".into());
            let pct = field(payload, "context_pct").unwrap_or_else(|| "?".into());
            let th = field(payload, "threshold").unwrap_or_else(|| "?".into());
            Some(format!("role={role} context={pct}% threshold={th}%"))
        }
        "queue.depth_high" => {
            let depth = field(payload, "depth").unwrap_or_else(|| "?".into());
            let th = field(payload, "threshold").unwrap_or_else(|| "?".into());
            let blocked = field(payload, "blocked_by").unwrap_or_else(|| "-".into());
            Some(format!("depth={depth}/{th} blocked_by={blocked}"))
        }
        "queue.starved" => {
            let depth = field(payload, "depth").unwrap_or_else(|| "?".into());
            let wait = field(payload, "head_wait_secs")
                .or_else(|| field(payload, "wait_secs"))
                .unwrap_or_else(|| "?".into());
            let blocked = field(payload, "blocked_by").unwrap_or_else(|| "-".into());
            Some(format!("depth={depth} head_wait={wait}s blocked_by={blocked}"))
        }
        _ => None,
    };
    let s = s.unwrap_or_else(|| generic_summary(payload));
    sanitize_line(&s, SUMMARY_MAX_BYTES)
}

fn generic_summary(payload: &Value) -> String {
    let Some(map) = payload.as_object() else {
        return scalar(payload).unwrap_or_default();
    };
    let mut keys: Vec<&String> = map
        .keys()
        .filter(|k| !SUMMARY_DENY_KEYS.contains(&k.as_str()))
        .collect();
    keys.sort();
    keys.iter()
        .filter_map(|k| map.get(*k).and_then(scalar).map(|v| format!("{k}={v}")))
        .take(4)
        .collect::<Vec<String>>()
        .join(" ")
}

/// 이벤트 봉투 → [`AlertItem`]. 대상이 아니면 `None`.
pub fn summarize(event: &Value) -> Option<AlertItem> {
    let name = event.get("name").and_then(|v| v.as_str())?;
    if !routable(name) {
        return None;
    }
    let surface = event.get("surface_id").and_then(|v| v.as_u64());
    let summary = summarize_payload(name, event.get("payload").unwrap_or(&Value::Null));
    Some(AlertItem { key: AlertKey::new(name, surface), summary })
}

/// 적재 문안 — 계약 서식 `[alert] <name> surface:<id> <요약 1줄>`(CONTRACTS §C).
/// 좌석 없는 경보는 `surface:-`. 선두 `[alert]` 는 스케줄 push 의 기계 라벨 규약
/// (`schedule::has_machine_label`)과 동형이라 판독자가 오너 입력과 구별할 수 있다.
pub fn render_text(item: &AlertItem, repeat: u64) -> String {
    let sid = item
        .key
        .surface
        .map(|s| s.to_string())
        .unwrap_or_else(|| "-".to_string());
    let mut t = format!("[alert] {} surface:{}", item.key.name, sid);
    if !item.summary.is_empty() {
        t.push(' ');
        t.push_str(&item.summary);
    }
    if repeat > 1 {
        t.push_str(&format!(" (반복 {repeat}건)"));
    }
    t
}

// ─────────────────────────── 데몬 결합부(비순수) ───────────────────────────

/// 살아있는 CSO 역할 좌석 전체(사전순 · 정확히 `cso` 인 좌석이 맨 앞).
/// 역할명 접두 규칙은 훅 `session-start.sh` 의 `cso*)` 와 같다. 생존 필터는
/// `system.resolve_role` 동형(roles 맵은 자력 종료 좌석을 스스로 비우지 않는다).
///
/// ★락 규율: `roles` 가드를 **놓은 뒤** `get_surface`(surfaces 락)를 잡는다. 반대로 하면
/// `close_surface`(surfaces → roles)와 AB-BA 데드락이 된다.
pub fn cso_seats(daemon: &Arc<Daemon>) -> Vec<u64> {
    let mut candidates: Vec<(String, u64)> = {
        let roles = daemon.roles.lock().unwrap();
        roles
            .iter()
            .filter(|(r, _)| r.starts_with("cso"))
            .map(|(r, s)| (r.clone(), *s))
            .collect()
    };
    candidates.sort_by(|a, b| (a.0 != "cso", &a.0).cmp(&(b.0 != "cso", &b.0)));
    candidates
        .into_iter()
        .filter(|(_, sid)| {
            daemon
                .get_surface(*sid)
                .is_some_and(|s| !s.exited.load(Ordering::Relaxed))
        })
        .map(|(_, sid)| sid)
        .collect()
}

/// 판정 재료를 데몬에서 뜬다(락은 각각 짧게 잡고 즉시 놓는다 — 어떤 락도 겹쳐 쥐지 않는다).
pub fn route_ctx(daemon: &Arc<Daemon>, now: f64) -> RouteCtx {
    let seats = cso_seats(daemon);
    let target = seats.first().copied();
    let paused = daemon.paused.load(Ordering::Relaxed);
    let (seat_paused, depth) = match target.and_then(|sid| daemon.get_surface(sid)) {
        Some(s) => {
            let seat_paused = s
                .queue_paused_until
                .lock()
                .unwrap()
                .is_some_and(|t| t > std::time::Instant::now());
            let depth = s.pending_queue.lock().unwrap().len();
            (seat_paused, depth)
        }
        None => (false, 0),
    };
    RouteCtx {
        now,
        daemon_started_at: daemon.started_at,
        cso_surface: target,
        cso_seats: seats,
        delivery_frozen: paused || seat_paused,
        cso_queue_depth: depth,
    }
}

/// 적재 실패 사유 — 전부 **보류로 되돌아간다**(폐기 아님).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueErr {
    /// 적재 시점에 CSO 좌석이 맵에서 사라졌거나 죽었다.
    SeatGone,
    /// 활성 큐가 보호선에 닿았다.
    QueueFull,
    /// 적재 직전에 배달이 동결됐다(판정 이후 pause 가 켜진 늦은 창).
    Frozen,
}

impl EnqueueErr {
    pub fn as_str(self) -> &'static str {
        match self {
            EnqueueErr::SeatGone => "seat_gone",
            EnqueueErr::QueueFull => "queue_full",
            EnqueueErr::Frozen => "delivery_frozen",
        }
    }
}

/// 좌석 활성 큐 적재의 **단일 지점** — alert 라우팅과 `via_queue` 스케줄 push 가 공유한다.
/// 기존 enqueue 3경로(handlers `surface.send_text --queued` · governance 승인 wakeup · 만료 통지)와
/// 동형이다: `next_queue_entry` → `pending_queue` → `queue.enqueued` → `persist_queue_state`.
///
/// 【늦은 적재의 무음 유실】 `close_surface` 는 surfaces 맵에서 좌석을 지운 **뒤** 큐를 비운다.
/// 그 사이에 예전 `Arc<Surface>` 로 밀어 넣으면 그 항목은 WAL 스냅샷(surfaces 순회)에도 폐기
/// 통지에도 잡히지 않고 사라진다. 그래서 **좌석 조회와 삽입을 surfaces 맵 락 한 임계영역 안에서**
/// 한다 — close 는 그 락을 잡아야 좌석을 뺄 수 있으므로 두 일은 서로 배타적이다. 락 순서는 전역
/// 규약(restored_queue → surfaces → pending_queue)과 같은 방향이다. publish·persist 는 임계영역
/// **밖**에서 한다(persist 는 스스로 같은 락들을 잡는다 — 안에서 부르면 재진입 데드락이다).
///
/// ★정직: 성공은 **메모리 큐 적재**의 성공이다. 내구성은 `Daemon::queue_wal_durable()` 이 따로
/// 말한다(`persist_queue_state` 는 실패를 반환형으로 알리지 않는다 — 기존 WAL 의 잔여 한계).
pub fn enqueue_into_seat(
    daemon: &Arc<Daemon>,
    sid: u64,
    text: String,
    from: Option<String>,
    origin: &str,
    cap: usize,
) -> Result<(String, usize), EnqueueErr> {
    let (entry, depth) = {
        let surfaces = daemon.surfaces.lock().unwrap();
        let surface = surfaces.get(&sid).cloned().ok_or(EnqueueErr::SeatGone)?;
        if surface.exited.load(Ordering::Relaxed) {
            return Err(EnqueueErr::SeatGone);
        }
        // ★판정과 적재 사이에 pause 가 켜지는 늦은 창을 여기서 한 번 더 닫는다(원자 1회 읽기).
        //   기존 배달 게이트는 그대로 pause 를 존중하므로 이것은 심층 방어다(면제가 아니다).
        if daemon.paused.load(Ordering::Relaxed) {
            return Err(EnqueueErr::Frozen);
        }
        let mut q = surface.pending_queue.lock().unwrap();
        if q.len() >= cap {
            return Err(EnqueueErr::QueueFull);
        }
        // TTL 은 **명시하지 않는다**(`None`) — 데몬 기본(`CYS_QUEUE_TTL_SECS` · 기본 6h)을 그대로
        // 상속한다. 여기서 6h 를 박으면 운영자가 조정한 TTL 을 이 경로만 무시하는 새 예외축이 생긴다.
        let entry = daemon.next_queue_entry(text, from, origin);
        q.push_back(entry.clone());
        (entry, q.len())
    };
    daemon.bus.publish(
        "queue.enqueued",
        "queue",
        Some(sid),
        crate::state::queue_enqueued_payload(&entry, depth, json!(entry.from), None),
    );
    // P7 큐 WAL — enqueue 를 디스크에 확정(어떤 pending_queue 락도 쥐지 않은 지점에서).
    daemon.persist_queue_state();
    Ok((entry.id, depth))
}

/// alert 전용 래퍼 — 보호선([`CSO_QUEUE_HEADROOM`])까지만 쓴다(활성 큐 상한 100의 나머지는
/// 사람·노드의 실제 보고 몫이다).
pub fn enqueue_alert(daemon: &Arc<Daemon>, cso_sid: u64, text: String) -> Result<String, EnqueueErr> {
    enqueue_into_seat(
        daemon,
        cso_sid,
        text,
        Some(ALERT_FROM.to_string()),
        ALERT_ORIGIN,
        CSO_QUEUE_HEADROOM,
    )
    .map(|(id, _)| id)
}

fn publish_route(daemon: &Arc<Daemon>, name: &str, payload: Value) {
    daemon.bus.publish(name, "alert_route", None, payload);
}

fn state_lock(daemon: &Arc<Daemon>) -> std::sync::MutexGuard<'_, RouteState> {
    daemon.alert_route.lock().unwrap_or_else(|e| e.into_inner())
}

/// 적재 성공 기록 + `alert_route.routed` 발행(수신·재평가 공용).
fn commit_routed(daemon: &Arc<Daemon>, item: &AlertItem, sid: u64, entry_id: &str, repeat: u64, now: f64, from_pending: bool) {
    state_lock(daemon).record_routed(&item.key, now);
    publish_route(
        daemon,
        "alert_route.routed",
        json!({"name": item.key.name, "surface_id": item.key.surface,
               "cso_surface": sid, "queue_entry_id": entry_id, "repeat": repeat,
               "from_pending": from_pending, "durable": daemon.queue_wal_durable()}),
    );
}

fn hold_reason_for(e: EnqueueErr) -> HoldReason {
    match e {
        EnqueueErr::SeatGone => HoldReason::NoCso,
        EnqueueErr::QueueFull => HoldReason::QueueHeadroom,
        EnqueueErr::Frozen => HoldReason::Paused,
    }
}

/// 판정 → 적재 → 기록의 한 사이클(신규 수신분). **`alert_route` 락은 판정과 기록에서 각각 짧게**
/// 잡고, 그 사이(적재)에는 놓는다 — 락을 쥔 채 `pending_queue`·`persist_queue_state` 를 부르면
/// 큐 계열 락 순서 규약 밖의 락쌍이 생긴다.
///
/// 반환: 적재했으면 `Some(entry_id)`.
pub fn route_once(daemon: &Arc<Daemon>, item: &AlertItem, now: f64) -> Option<String> {
    let ctx = route_ctx(daemon, now);
    let (verdict, pending_count) = {
        let st = state_lock(daemon);
        (
            decide(&st, &item.key, &ctx),
            st.pending.get(&item.key).map(|p| p.count).unwrap_or(0),
        )
    };
    match verdict {
        Verdict::Ignore(r) => {
            let announce = r == IgnoreReason::CsoOwnSurface
                && state_lock(daemon).should_publish_ignored(&item.key, now);
            if announce {
                // 침묵 금지: 자기 이벤트 제외도 관측 가능한 사실로 남긴다. 다만 발행 자체에
                // 쿨다운을 걸어 관측이 소음이 되지 않게 한다(위 `last_ignored` 주석).
                publish_route(
                    daemon,
                    "alert_route.ignored",
                    json!({"name": item.key.name, "surface_id": item.key.surface,
                           "reason": r.as_str()}),
                );
            }
            None
        }
        Verdict::Hold(reason) => {
            hold(daemon, item, reason, now);
            None
        }
        Verdict::Route => {
            // 판정이 Route 인데 대상이 없을 수는 없다(④가 먼저 걸린다). 방어적으로 보류.
            let Some(sid) = ctx.cso_surface else {
                hold(daemon, item, HoldReason::NoCso, now);
                return None;
            };
            let repeat = pending_count + 1;
            let text = render_text(item, repeat);
            match enqueue_alert(daemon, sid, text) {
                Ok(entry_id) => {
                    commit_routed(daemon, item, sid, &entry_id, repeat, now, pending_count > 0);
                    Some(entry_id)
                }
                Err(e) => {
                    // ★적재 실패는 **보류**다 — 이때 pending 항목이 아직 없을 수 있으므로
                    //   `record_hold` 가 생성까지 한다(유지만 하면 그 사실이 사라진다).
                    hold(daemon, item, hold_reason_for(e), now);
                    None
                }
            }
        }
    }
}

fn hold(daemon: &Arc<Daemon>, item: &AlertItem, reason: HoldReason, now: f64) {
    let folded = state_lock(daemon).record_hold(&item.key, &item.summary, reason, now);
    if let Some(v) = folded {
        // 상한 초과 접기 — **조용히 버리지 않는다**(정직한 한계의 관측점).
        publish_route(
            daemon,
            "alert_route.pending_folded",
            json!({"name": v.name, "surface_id": v.surface, "limit": PENDING_MAX}),
        );
    }
}

/// 이벤트 1건 처리(구독 루프·검체 공용 진입점).
pub fn handle_event(daemon: &Arc<Daemon>, event: &Value, now: f64) {
    let Some(item) = summarize(event) else {
        return;
    };
    route_once(daemon, &item, now);
}

/// 재평가 — 유예 종료·CSO 착석·쿨다운 만료·상한 창 이동으로 **보류가 풀렸는지** 다시 본다.
/// 오래 기다린 것부터(first_seen 오름차순) 훑는다.
///
/// ★훑기 상한([`REEVAL_SCAN_MAX`])과 적재 상한([`HOURLY_CAP`])은 **다른 축**이다: 쿨다운 중인
/// 앞쪽 키는 예산을 쓰지 않고 지나가고(키 지역 사유), 전역 사유(유예·동결·부재·상한·큐 보호선)를
/// 만나면 그 자리에서 순회를 끝낸다(뒤 키도 같은 답을 받는다).
///
/// ★재평가는 **보류를 다시 세지 않는다** — 같은 항목이 매 틱 suppressed 카운터를 부풀리면
/// 관측이 거짓말을 한다(보류는 처음 한 번 세었다).
///
/// 반환: 이번 틱에 적재된 건수.
pub fn reevaluate(daemon: &Arc<Daemon>, now: f64) -> usize {
    let batch: Vec<(AlertKey, PendingAlert)> = {
        let st = state_lock(daemon);
        let mut v: Vec<(AlertKey, PendingAlert)> =
            st.pending.iter().map(|(k, p)| (k.clone(), p.clone())).collect();
        v.sort_by(|a, b| {
            a.1.first_seen
                .partial_cmp(&b.1.first_seen)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        v.truncate(REEVAL_SCAN_MAX);
        v
    };
    let mut routed = 0usize;
    for (key, p) in batch {
        // 보류분 재적재는 **키당 정확히 1건**이다(폭풍 N건이 N줄이 되지 않는다 — 병합 count 는
        // 문안의 `(반복 N건)` 으로만 실린다).
        let item = AlertItem { key, summary: p.summary.clone() };
        let ctx = route_ctx(daemon, now);
        let verdict = {
            let st = state_lock(daemon);
            decide(&st, &item.key, &ctx)
        };
        match verdict {
            Verdict::Route => {
                let Some(sid) = ctx.cso_surface else { break };
                let repeat = p.count.max(1);
                match enqueue_alert(daemon, sid, render_text(&item, repeat)) {
                    Ok(entry_id) => {
                        commit_routed(daemon, &item, sid, &entry_id, repeat, now, true);
                        routed += 1;
                    }
                    // 적재 실패 — pending 에 **그대로 남는다**(재보류 기록 없음). 좌석·큐 상태는
                    // 전역 사정이므로 이번 틱은 여기서 끝낸다.
                    Err(_) => break,
                }
            }
            Verdict::Ignore(_) => {
                // 자기 좌석 이벤트가 된 보류분(그 좌석이 CSO 로 승계) — 더는 대상이 아니다.
                state_lock(daemon).pending.remove(&item.key);
            }
            Verdict::Hold(r) if r.is_global() => break,
            Verdict::Hold(_) => {} // 쿨다운 = 키 지역 사유 — 다음 키를 본다
        }
    }
    routed
}

/// env 롤백 판정(순수) — 명시적 거짓만 끈다(미설정=켬).
pub fn enabled_from(raw: Option<&str>) -> bool {
    !matches!(
        raw.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("off") | Some("no")
    )
}

/// 이벤트 처리 1건을 패닉 격리로 감싼다(구독 루프 전 구간 공용).
fn guarded(daemon: &Arc<Daemon>, event: &Value, whence: &'static str) {
    let d = Arc::clone(daemon);
    let body = std::panic::AssertUnwindSafe(|| handle_event(&d, event, now_epoch()));
    if std::panic::catch_unwind(body).is_err() {
        publish_route(daemon, "alert_route.panic", json!({"where": whence}));
    }
}

/// 구독 태스크 기동 — `boot_supervisor::spawn` 규약 동형(별도 tokio 태스크·자기 cadence·
/// 패닉 격리). `main.rs` 배선은 `spawn_scheduler` 뒤·`channels::reconcile` 앞.
///
/// **멱등**: 이미 떠 있으면 아무것도 하지 않는다(단일 실행자 불변식 — 둘이 돌면 상한이 깨진다).
pub fn spawn(daemon: Arc<Daemon>) {
    if !enabled_from(std::env::var(ENV_ALERT_ROUTE).ok().as_deref()) {
        eprintln!("[cysd] alert-route disabled ({ENV_ALERT_ROUTE}=0) — 경보는 CSO 큐로 가지 않는다");
        return;
    }
    {
        let mut st = state_lock(&daemon);
        if st.enabled {
            eprintln!("[cysd] alert-route: 이미 기동됨 — 중복 구독을 거절한다(단일 실행자)");
            return;
        }
        st.enabled = true;
    }
    tokio::spawn(async move {
        // ★구독을 **replay 보다 먼저** 연다(run_event_stream 규약) — 그 사이에 발행된 이벤트가
        //   두 경로 어디에도 없는 갭으로 떨어지지 않게. 중복은 seq 커서로 거른다.
        let mut rx = daemon.bus.subscribe();
        let mut cursor: u64 = 0;
        // 부트 시점 ring 잔여분도 훑는다(태스크 기동 전 발행분). 어차피 유예 창 안이라 대부분
        // 보류로 가지만, 폐기하지 않는 것이 이 모듈의 계약이다. ★이 구간은 갭으로 보고하지
        // 않는다 — 데몬이 뜨기 전의 seq 는 '유실' 이 아니라 '우리 이전' 이다(영속 seq 는
        // 이벤트 본문이 아니라 예약 상한이라 그 차이를 유실 건수로 세면 허위 관측이 된다).
        for event in daemon.bus.replay_after(cursor) {
            cursor = event["seq"].as_u64().unwrap_or(cursor).max(cursor);
            guarded(&daemon, &event, "boot_replay");
        }
        if cursor == 0 {
            cursor = daemon.bus.latest_seq();
        }
        let mut tick = tokio::time::interval(Duration::from_secs(REEVAL_INTERVAL_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await; // 첫 tick 즉시 발화분 소비
        loop {
            tokio::select! {
                r = rx.recv() => match r {
                    Ok(event) => {
                        let seq = event["seq"].as_u64().unwrap_or(0);
                        if seq <= cursor {
                            continue; // 이미 본 것(replay 중복)
                        }
                        cursor = seq;
                        guarded(&daemon, &event, "live");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // ★느린 소비자 — **구독을 끊지 않는다**(끊으면 그 뒤 경보가 영영 안 온다).
                        //   ring 에서 커서 이후를 재생해 메운다.
                        let replayed = daemon.bus.replay_after(cursor);
                        // ★갭은 **실제 재생 결과**로 판정한다(먼저 읽은 bounds 는 그 사이 또
                        //   퇴출될 수 있어 무누락의 근거가 되지 못한다). 재생분의 첫 seq 가
                        //   cursor+1 보다 크면 그 사이가 복구 불가 구간이다.
                        let first = replayed.first().and_then(|e| e["seq"].as_u64());
                        let latest = daemon.bus.latest_seq();
                        let lost_until = match first {
                            Some(f) if f > cursor + 1 => Some(f - 1),
                            None if latest > cursor => Some(latest),
                            _ => None,
                        };
                        if let Some(until) = lost_until {
                            publish_route(&daemon, "alert_route.replay_gap",
                                json!({"lagged": n, "from": cursor + 1, "lost_until": until,
                                       "alerts_lost": Value::Null,
                                       "note": "ring 퇴출 구간은 복구 불가 — 그 구간에 경보가 몇 건 있었는지는 알 수 없다"}));
                        }
                        for event in replayed {
                            cursor = event["seq"].as_u64().unwrap_or(cursor).max(cursor);
                            guarded(&daemon, &event, "replay");
                        }
                    }
                    Err(_) => return, // 버스 종료 = 데몬 종료
                },
                _ = tick.tick() => {
                    let d = Arc::clone(&daemon);
                    let body = std::panic::AssertUnwindSafe(|| { reevaluate(&d, now_epoch()); });
                    if std::panic::catch_unwind(body).is_err() {
                        publish_route(&daemon, "alert_route.panic", json!({"where": "reevaluate"}));
                    }
                }
            }
        }
    });
}

// ═══════════════════════════ 드릴(데몬 결합 · PTY 필요) ═══════════════════════════
// ★`#[cfg(unix)]`: 좌석 생성이 실제 PTY 를 띄운다(`sleep 30`). 순수층 검체는 이 게이트 **밖**에
//   있어 Windows CI 에서도 돈다 — 폭풍 억제의 판정 규칙은 그쪽이 전부 봉인한다.
#[cfg(all(test, unix))]
mod drills {
    use super::*;
    use serde_json::json;

    fn drill_daemon(tag: &str) -> Arc<Daemon> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cys-alertdrill-{}-{}-{}-{}",
            tag,
            std::process::id(),
            now_epoch() as u64,
            n
        ));
        let _ = std::fs::create_dir_all(&dir);
        Daemon::new(dir.join("cysd.sock"))
    }

    /// 역할 좌석 하나(live PTY) — governance 드릴의 `spawn_role_surface` 와 동형.
    fn seat(daemon: &Arc<Daemon>, role: &str) -> u64 {
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some(role.into()), 24, 80)
            .expect("create surface");
        daemon.roles.lock().unwrap().insert(role.into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.id
    }

    fn ev(name: &str, sid: Option<u64>, payload: Value) -> Value {
        json!({"type": "event", "seq": 1, "name": name, "category": "test",
               "surface_id": sid, "payload": payload})
    }

    fn depth(daemon: &Arc<Daemon>, sid: u64) -> usize {
        daemon
            .get_surface(sid)
            .map(|s| s.pending_queue.lock().unwrap().len())
            .unwrap_or(0)
    }

    fn pending_len(daemon: &Arc<Daemon>) -> usize {
        daemon.alert_route.lock().unwrap().pending.len()
    }

    /// 유예(300s)를 지난 시각 — 드릴은 벽시계를 기다리지 않고 `now` 를 인자로 민다.
    fn after_grace(daemon: &Arc<Daemon>) -> f64 {
        daemon.started_at + BOOT_GRACE_SECS + 100.0
    }

    /// ★폭풍 봉인 ①(§7): 같은 사실 100건이 CSO 큐에 100줄이 되지 않는다 — 쿨다운이 1건만
    /// 통과시키고 나머지 99건은 **버려지지 않고** 키 하나로 접혀 보관되며, 쿨다운이 풀린 뒤
    /// 재평가가 그것을 **정확히 1건**으로 적재한다(폐기 0 · 폭주 0 을 한 검체가 함께 증명한다).
    #[test]
    fn drill_storm_of_one_key_routes_one_then_reevaluates_to_exactly_one() {
        let daemon = drill_daemon("storm-one");
        let cso = seat(&daemon, "cso");
        let worker = seat(&daemon, "worker");
        let now = after_grace(&daemon);
        for _ in 0..100 {
            handle_event(&daemon, &ev("health.alert", Some(worker), json!({"rule": "panic"})), now);
        }
        assert_eq!(depth(&daemon, cso), 1, "폭풍 100건이 큐에 1줄 — 쿨다운이 나머지를 막았다");
        assert_eq!(pending_len(&daemon), 1, "억제분은 키 하나로 병합돼 보관된다");
        {
            let st = daemon.alert_route.lock().unwrap();
            let p = st.pending.values().next().unwrap();
            assert_eq!(p.count, 99, "99건이 한 키에 병합(폐기 0)");
            assert_eq!(p.reason, "cooldown");
            assert_eq!(st.suppressed_1h(now), 99);
            assert_eq!(st.routed_1h(now), 1);
        }
        // 쿨다운 중 재평가는 아무것도 적재하지 않는다(억제는 억제다).
        assert_eq!(reevaluate(&daemon, now), 0);
        assert_eq!(depth(&daemon, cso), 1);
        // 쿨다운 만료 후 재평가 — **1건**만 적재된다(99줄이 아니다).
        let later = now + COOLDOWN_SECS + 1.0;
        assert_eq!(reevaluate(&daemon, later), 1);
        assert_eq!(depth(&daemon, cso), 2);
        assert_eq!(pending_len(&daemon), 0, "재평가된 보류분은 큐로 갔다");
        let text = daemon
            .get_surface(cso)
            .unwrap()
            .pending_queue
            .lock()
            .unwrap()
            .back()
            .unwrap()
            .text
            .clone();
        assert!(text.contains("(반복 99건)"), "병합 건수는 문안에 1줄로 실린다: {text}");
    }

    /// ★폭풍 봉인 ②: 서로 다른 100키(쿨다운이 안 걸리는 최악)는 **시간당 상한 20**에서 멈춘다.
    /// 나머지 80은 보류로 보관된다(0건 폐기).
    #[test]
    fn drill_storm_of_distinct_keys_stops_at_hourly_cap_and_keeps_the_rest() {
        let daemon = drill_daemon("storm-many");
        let cso = seat(&daemon, "cso");
        let now = after_grace(&daemon);
        for i in 0..100u64 {
            handle_event(
                &daemon,
                &ev("health.alert", Some(9000 + i), json!({"rule": format!("r{i}")})),
                now,
            );
        }
        assert_eq!(depth(&daemon, cso), HOURLY_CAP, "상한 20건에서 적재가 멈춘다");
        assert_eq!(pending_len(&daemon), 100 - HOURLY_CAP, "나머지는 전부 보관(폐기 0)");
        // 상한 창 안에서는 재평가도 더 밀지 못한다.
        assert_eq!(reevaluate(&daemon, now + 1.0), 0);
        assert_eq!(depth(&daemon, cso), HOURLY_CAP);
    }

    /// ★pause(kill-switch) 중에는 **0건**이다 — 그리고 0건은 폐기가 아니라 보관이다.
    /// 해제 뒤 재평가가 그대로 복구한다(§8: 새 신호가 pause 게이트를 면제하지 않는다).
    #[test]
    fn drill_paused_daemon_routes_nothing_and_recovers_after_resume() {
        let daemon = drill_daemon("pause");
        let cso = seat(&daemon, "cso");
        let now = after_grace(&daemon);
        daemon.paused.store(true, Ordering::Relaxed);
        for i in 0..10u64 {
            handle_event(&daemon, &ev("health.alert", Some(500 + i), json!({"rule": "x"})), now);
        }
        assert_eq!(depth(&daemon, cso), 0, "pause 중 적재 0건");
        assert_eq!(pending_len(&daemon), 10, "0건은 폐기가 아니라 보관이다");
        daemon.paused.store(false, Ordering::Relaxed);
        assert_eq!(reevaluate(&daemon, now), 10, "해제 뒤 재평가가 전부 복구");
        assert_eq!(depth(&daemon, cso), 10);
        assert_eq!(pending_len(&daemon), 0);
    }

    /// ★CSO 부재는 폐기 사유가 아니다 — `context.threshold` 는 에지 1회 발행이라 여기서 버리면
    /// 컨텍스트 60% 초과가 **영영** 전달되지 않는다(치명위험 ②의 직접 경로).
    #[test]
    fn drill_no_cso_keeps_alerts_until_a_seat_appears() {
        let daemon = drill_daemon("nocso");
        let worker = seat(&daemon, "worker");
        let now = after_grace(&daemon);
        handle_event(
            &daemon,
            &ev("context.threshold", Some(worker), json!({"role": "worker", "context_pct": 62, "threshold": 60})),
            now,
        );
        assert_eq!(pending_len(&daemon), 1, "CSO 가 없어도 사실은 남는다");
        assert_eq!(
            daemon.alert_route.lock().unwrap().pending.values().next().unwrap().reason,
            "no_cso"
        );
        let cso = seat(&daemon, "cso");
        assert_eq!(reevaluate(&daemon, now), 1, "착석하면 그때 배달된다");
        let text = daemon.get_surface(cso).unwrap().pending_queue.lock().unwrap()[0].text.clone();
        assert!(text.starts_with("[alert] context.threshold surface:"), "{text}");
        assert!(text.contains("context=62%"), "요약이 사실을 담는다: {text}");
    }

    /// ★자기 이벤트 제외 — CSO 자신의 좌석에서 난 경보를 CSO 큐에 넣으면 그 적체가 다시 경보를
    /// 낳는 되먹임이 된다. 제외분은 보류에도 남기지 않는다(되돌아올 상태가 아니다).
    #[test]
    fn drill_cso_own_surface_events_are_excluded_and_not_kept() {
        let daemon = drill_daemon("selfex");
        let cso = seat(&daemon, "cso");
        let now = after_grace(&daemon);
        for name in ["queue.depth_high", "queue.starved", "health.alert"] {
            handle_event(&daemon, &ev(name, Some(cso), json!({"depth": 60})), now);
        }
        assert_eq!(depth(&daemon, cso), 0, "자기 좌석 이벤트는 적재하지 않는다");
        assert_eq!(pending_len(&daemon), 0, "보류에도 남기지 않는다(무한 보관 금지)");
    }

    /// ★CSO 좌석이 둘일 때(cso·cso-2) **양쪽 다** 자기제외 대상이다 — 아니면 A 의 적체를 B 에게,
    /// B 의 적체를 A 에게 미는 순환이 생긴다.
    #[test]
    fn drill_all_cso_seats_are_excluded_not_only_the_target() {
        let daemon = drill_daemon("twocso");
        let cso = seat(&daemon, "cso");
        let cso2 = seat(&daemon, "cso-2");
        let now = after_grace(&daemon);
        handle_event(&daemon, &ev("queue.depth_high", Some(cso2), json!({"depth": 60})), now);
        assert_eq!(depth(&daemon, cso), 0, "다른 CSO 좌석의 적체도 CSO 큐로 보내지 않는다");
        assert_eq!(pending_len(&daemon), 0);
    }

    /// ★적재 항목의 계약(CONTRACTS §C): origin="alert" · from=Some("daemon") ·
    /// text="[alert] <name> surface:<id> <요약>" · TTL 은 데몬 기본(6h) 상속.
    #[test]
    fn drill_alert_entry_shape_and_ttl_contract() {
        let daemon = drill_daemon("shape");
        let cso = seat(&daemon, "cso");
        let worker = seat(&daemon, "worker");
        let now = after_grace(&daemon);
        handle_event(
            &daemon,
            &ev("surface.exited", Some(worker), json!({"role": "worker", "agent": "claude"})),
            now,
        );
        let e = daemon.get_surface(cso).unwrap().pending_queue.lock().unwrap()[0].clone();
        assert_eq!(e.origin, "alert");
        assert_eq!(e.from.as_deref(), Some("daemon"));
        assert_eq!(
            e.text,
            format!("[alert] surface.exited surface:{worker} role=worker agent=claude")
        );
        assert!(e.ttl_secs.is_none(), "TTL 은 데몬 기본을 상속한다(경보만의 예외축 금지)");
        // 기본 TTL(6h)이 실제로 적용되는지 — 6h 전은 살아 있고 6h 뒤는 만료다.
        let ttl = crate::state::queue_ttl_default_secs();
        assert_eq!(ttl, 6 * 3600, "정본 alert TTL 6h = 데몬 기본값");
        let t0 = e.enqueued_at;
        assert!(!crate::state::queue_entry_expired(&e, t0 + ttl as f64 - 1.0, ttl));
        assert!(crate::state::queue_entry_expired(&e, t0 + ttl as f64 + 1.0, ttl));
    }

    /// ★CSO 활성 큐 보호선 — 경보가 업무 메시지 몫(활성 큐 상한 100의 절반)을 먹지 않는다.
    /// 보호선에 닿으면 적재하지 않고 **보관**한다.
    #[test]
    fn drill_queue_headroom_holds_instead_of_filling_the_seat() {
        let daemon = drill_daemon("headroom");
        let cso = seat(&daemon, "cso");
        let now = after_grace(&daemon);
        {
            let s = daemon.get_surface(cso).unwrap();
            let mut q = s.pending_queue.lock().unwrap();
            for i in 0..CSO_QUEUE_HEADROOM {
                q.push_back(daemon.next_queue_entry(format!("[보고] 업무 {i}"), None, "send"));
            }
        }
        handle_event(&daemon, &ev("health.alert", Some(77), json!({"rule": "x"})), now);
        assert_eq!(depth(&daemon, cso), CSO_QUEUE_HEADROOM, "보호선 너머로 밀지 않는다");
        assert_eq!(pending_len(&daemon), 1);
        assert_eq!(
            daemon.alert_route.lock().unwrap().pending.values().next().unwrap().reason,
            "queue_headroom"
        );
    }

    /// ★부트 유예 300초 — 기동 폭풍이 CSO 좌석을 덮지 않는다. 그리고 유예분도 보관된다.
    #[test]
    fn drill_boot_grace_holds_everything_and_releases_after() {
        let daemon = drill_daemon("grace");
        let cso = seat(&daemon, "cso");
        let inside = daemon.started_at + 10.0;
        handle_event(&daemon, &ev("health.alert", Some(42), json!({"rule": "boot"})), inside);
        assert_eq!(depth(&daemon, cso), 0, "유예 창 안에서는 적재 0");
        assert_eq!(pending_len(&daemon), 1);
        assert_eq!(reevaluate(&daemon, daemon.started_at + BOOT_GRACE_SECS + 1.0), 1);
        assert_eq!(depth(&daemon, cso), 1);
    }

    /// ★비대상 이벤트는 상태를 전혀 건드리지 않는다(이 모듈이 스스로 발행하는 이벤트 포함 —
    /// 되먹임 0). `queue.enqueued` 는 적재할 때마다 나가므로 이것이 routable 이면 곧 폭주다.
    #[test]
    fn drill_own_and_unrelated_events_do_not_enter_the_router() {
        let daemon = drill_daemon("norecur");
        let cso = seat(&daemon, "cso");
        let now = after_grace(&daemon);
        for name in ["queue.enqueued", "alert_route.routed", "queue.delivered", "health.action"] {
            handle_event(&daemon, &ev(name, Some(1234), json!({"x": 1})), now);
        }
        assert_eq!(depth(&daemon, cso), 0);
        assert_eq!(pending_len(&daemon), 0);
        assert_eq!(daemon.alert_route.lock().unwrap().suppressed_total, 0);
    }

    /// ★죽은 좌석에 늦게 적재하지 않는다 — surfaces 맵에서 빠진 좌석에 밀어 넣으면 그 항목은
    /// WAL·폐기 통지 어디에도 잡히지 않고 사라진다(무음 유실). 그런 경우는 **보류**여야 한다.
    #[test]
    fn drill_dead_cso_seat_holds_instead_of_losing_the_alert() {
        let daemon = drill_daemon("deadseat");
        let cso = seat(&daemon, "cso");
        let now = after_grace(&daemon);
        // 역할 매핑은 남기고 좌석만 죽인다(자력 종료 = roles 맵이 스스로 비지 않는 실제 형상).
        daemon
            .get_surface(cso)
            .unwrap()
            .exited
            .store(true, Ordering::Relaxed);
        handle_event(&daemon, &ev("health.alert", Some(31), json!({"rule": "y"})), now);
        assert_eq!(depth(&daemon, cso), 0, "죽은 좌석 큐에 넣지 않는다");
        assert_eq!(pending_len(&daemon), 1, "그 사실은 보관된다");
        assert_eq!(
            daemon.alert_route.lock().unwrap().pending.values().next().unwrap().reason,
            "no_cso"
        );
    }

    /// ★`enqueue_into_seat` 의 원자성 계약(회귀 핀): 좌석 조회·생존 판정·삽입이 **surfaces 맵
    /// 락 한 임계영역** 안에 있어야 한다. 이 배선이 풀리면 close 와의 경쟁에서 항목이 조용히
    /// 사라진다(그 실패는 런타임 경쟁이라 단위 검체로 재현이 어려워 소스 핀으로 박제한다).
    #[test]
    fn source_pin_enqueue_holds_surfaces_lock_across_lookup_and_insert() {
        let src = include_str!("alert_route.rs");
        let at = src
            .find("pub fn enqueue_into_seat(")
            .expect("enqueue_into_seat 소실");
        let body = &src[at..at + 2200];
        let lock = body
            .find("let surfaces = daemon.surfaces.lock().unwrap();")
            .expect("surfaces 맵 락을 잡지 않는다 — close 와의 경쟁에서 무음 유실");
        let push = body.find("q.push_back(entry.clone());").expect("삽입 소실");
        assert!(lock < push, "삽입이 surfaces 임계영역 밖으로 나갔다");
        let get = body.find("surfaces.get(&sid)").expect("맵 조회 소실");
        assert!(lock < get && get < push, "조회와 삽입이 같은 임계영역에 있어야 한다");
        // persist 는 임계영역 **밖**(재진입 데드락 방지) — 블록 종료 뒤에 있어야 한다.
        let persist = body.find("daemon.persist_queue_state();").expect("persist 소실");
        let block_end = body.find("    };").expect("임계영역 종료 소실");
        assert!(persist > block_end, "persist 가 락 안으로 들어갔다(재진입 데드락)");
    }
}

// ═══════════ 순수층 검체(PTY 불요 · Windows CI 에서도 돈다) ═══════════
// ★출처: codex(gpt-6-astra) 위임 작성분을 **전 줄 검토 후** 채택했다(2026-09-07).
//   검토에서 하나가 실제 결함을 잡았다 — `sanitize_line` 이 Cf(투명문자·ZWSP)를 통과시켰다.
//   그 검체는 그대로 두고 **구현을 고쳤다**(검체를 구현에 맞추지 않는다).
#[cfg(test)]
mod pure_tests {
    use super::*;
    use serde_json::json;

    fn ctx(now: f64) -> RouteCtx {
        RouteCtx {
            now,
            daemon_started_at: 0.0,
            cso_surface: Some(7),
            cso_seats: vec![7],
            delivery_frozen: false,
            cso_queue_depth: 0,
        }
    }

    fn key() -> AlertKey {
        AlertKey::new("health.alert", Some(8))
    }

    fn gate_fixture(gates: [bool; 6]) -> (RouteState, RouteCtx) {
        let mut state = RouteState::default();
        let mut context = ctx(10_000.0);
        if gates[0] {
            context.daemon_started_at = context.now - 299.9;
        }
        context.delivery_frozen = gates[1];
        if gates[2] {
            context.cso_surface = None;
        }
        if gates[3] {
            state.last_routed.insert(key(), context.now - 1.0);
        }
        if gates[4] {
            state.routed_window.extend(std::iter::repeat(context.now).take(HOURLY_CAP));
        }
        if gates[5] {
            context.cso_queue_depth = CSO_QUEUE_HEADROOM;
        }
        (state, context)
    }

    fn assert_hold_priority(index: usize) {
        let reasons = [HoldReason::BootGrace, HoldReason::Paused, HoldReason::NoCso,
            HoldReason::Cooldown, HoldReason::HourlyCap, HoldReason::QueueHeadroom];
        let mut gates = [false; 6];
        gates[index..].fill(true);
        let (state, context) = gate_fixture(gates);
        assert_eq!(decide(&state, &key(), &context), Verdict::Hold(reasons[index]),
            "보류 우선순위가 깨졌다: 뒤 조건보다 {:?}가 먼저여야 한다", reasons[index]);
        for earlier in 0..index {
            gates[earlier] = true;
            let (state, context) = gate_fixture(gates);
            assert_eq!(decide(&state, &key(), &context), Verdict::Hold(reasons[earlier]),
                "보류 우선순위가 깨졌다: {:?}가 {:?}를 가려야 한다", reasons[earlier], reasons[index]);
            gates[earlier] = false;
        }
        assert_eq!(decide(&state, &AlertKey::new("queue.enqueued", Some(7)), &context),
            Verdict::Ignore(IgnoreReason::NotRoutable), "비대상 제외가 자기좌석 제외와 보류보다 뒤로 밀렸다");
        assert_eq!(decide(&state, &AlertKey::new("health.alert", Some(7)), &context),
            Verdict::Ignore(IgnoreReason::CsoOwnSurface), "자기좌석 제외가 보류보다 뒤로 밀렸다");
    }

    // ★관측이 소음이 되지 않게: 자기 좌석 제외의 **발행**에도 쿨다운 1개 창을 건다.
    //   헬스 룰 디바운스(30s)만 타고 오는 CSO 좌석 경보마다 `alert_route.ignored` 를 쏘면
    //   룰 10종에서 시간당 1,200줄이 버스로 나간다 — 관측이 관측 대상을 오염시킨다.
    //   판정은 이 맵을 보지 않는다(제외는 시간이 지나도 제외다).
    #[test]
    fn ignored_observation_is_rate_limited_but_judgement_is_not() {
        let mut state = RouteState::default();
        let k = AlertKey::new("health.alert", Some(7));
        assert!(state.should_publish_ignored(&k, 10_000.0), "첫 제외는 보여야 한다");
        assert!(!state.should_publish_ignored(&k, 10_299.9), "쿨다운 안에서 또 쏘면 소음이다");
        assert!(state.should_publish_ignored(&k, 10_300.0), "쿨다운이 지나면 다시 보인다");
        // 다른 키는 서로의 창을 쓰지 않는다.
        assert!(state.should_publish_ignored(&AlertKey::new("health.alert", Some(8)), 10_300.0));
        // 판정 자체는 발행 쿨다운과 무관하다 — 자기 좌석 이벤트는 언제나 제외다.
        let context = ctx(10_300.0);
        for t in [10_300.0, 10_300.1, 20_000.0] {
            assert_eq!(
                decide(&state, &AlertKey::new("health.alert", Some(7)), &ctx(t)),
                Verdict::Ignore(IgnoreReason::CsoOwnSurface),
                "발행 쿨다운이 판정을 흔들었다"
            );
        }
        assert_eq!(context.cso_seats, vec![7]);
    }

    // 자기 발행 이벤트를 다시 입력으로 받아 경보가 증식하는 되먹임을 막는 핀이다.
    #[test]
    fn emitted_events_are_never_routable() {
        for name in ["alert_route.routed", "alert_route.ignored", "alert_route.pending_folded",
            "alert_route.replay_gap", "alert_route.panic", "queue.enqueued", "queue.delivered"] {
            assert!(!routable(name), "되먹임 금지가 깨졌다: {name}이 다시 라우팅된다");
        }
    }

    // 허용 목록 자체가 닫히거나 watchdog 접두 판정이 넓어지는 회귀를 막는다.
    #[test]
    fn routable_names_follow_the_allowlist() {
        for name in ["health.alert", "surface.exited", "context.threshold", "queue.starved",
            "queue.depth_high", "watchdog.load_high", "watchdog.custom", OVERFLOW_NAME] {
            assert!(routable(name), "허용 목록의 경보 {name}이 차단됐다");
        }
        for name in ["", "watchdog", "watchdogs.load_high", "health.alert.extra"] {
            assert!(!routable(name), "허용 목록 밖 이름 {name}이 라우팅된다");
        }
    }

    // 결측끼리의 일치를 자기좌석으로 오인해 CSO 부재 경보를 폐기하지 못하게 한다.
    #[test]
    fn missing_surface_is_held_instead_of_ignored() {
        let state = RouteState::default();
        let mut context = ctx(10_000.0);
        context.cso_surface = None;
        context.cso_seats.clear();
        assert_eq!(decide(&state, &AlertKey::new("health.alert", None), &context),
            Verdict::Hold(HoldReason::NoCso), "결측 좌석 경보가 부재 보류 대신 폐기됐다");
        context.cso_seats = vec![7];
        assert_eq!(decide(&state, &AlertKey::new("health.alert", Some(7)), &context),
            Verdict::Ignore(IgnoreReason::CsoOwnSurface), "실재하는 CSO 좌석의 자기 경보가 제외되지 않았다");
        assert_eq!(decide(&state, &key(), &context), Verdict::Hold(HoldReason::NoCso),
            "다른 좌석의 경보가 자기 경보로 오인됐다");
    }

    // 제외 조건 뒤에서 부트 유예가 모든 보류보다 우선하며 정확히 300초에 풀리는 핀이다.
    #[test]
    fn boot_grace_has_first_hold_priority() {
        assert_hold_priority(0);
        assert_eq!(decide(&RouteState::default(), &key(), &ctx(300.0)), Verdict::Route,
            "부트 유예가 정확히 300초에 해제되지 않았다");
    }

    // 동결은 유예가 없을 때만 나타나고 부재 및 용량 사유보다 우선해야 한다.
    #[test]
    fn paused_has_second_hold_priority() { assert_hold_priority(1); }

    // CSO 부재는 유예와 동결 뒤, 키 쿨다운과 용량 사유 앞에서 보류해야 한다.
    #[test]
    fn no_cso_has_third_hold_priority() { assert_hold_priority(2); }

    // 키 쿨다운이 전역 선행 조건을 덮지 않고 시간당 상한보다 먼저 드러나게 한다.
    #[test]
    fn cooldown_has_fourth_hold_priority() { assert_hold_priority(3); }

    // 시간당 상한은 선행 네 조건이 없을 때만 큐 보호선보다 우선해야 한다.
    #[test]
    fn hourly_cap_has_fifth_hold_priority() { assert_hold_priority(4); }

    // 큐 보호선은 마지막 보류이며 보호선 바로 아래에서는 라우팅을 허용해야 한다.
    #[test]
    fn queue_headroom_has_last_hold_priority() {
        assert_hold_priority(5);
        let mut context = ctx(10_000.0);
        context.cso_queue_depth = CSO_QUEUE_HEADROOM - 1;
        assert_eq!(decide(&RouteState::default(), &key(), &context), Verdict::Route,
            "큐 보호선 미만인데 경보가 보류됐다");
    }

    // 쿨다운의 미만 비교를 핀으로 고정해 300초가 지난 경보의 불필요한 보류를 막는다.
    #[test]
    fn cooldown_opens_at_exactly_three_hundred_seconds() {
        let mut state = RouteState::default();
        state.record_routed(&key(), 10_000.0);
        assert_eq!(decide(&state, &key(), &ctx(10_299.9)), Verdict::Hold(HoldReason::Cooldown),
            "299.9초 만에 같은 경보가 다시 허용됐다");
        assert_eq!(decide(&state, &key(), &ctx(10_300.0)), Verdict::Route,
            "300초 경계에서 쿨다운이 풀리지 않았다");
    }

    // 쿨다운과 무관한 새 키로 시간당 상한이 창 이동에 따라 해제되는지 지킨다.
    #[test]
    fn hourly_cap_expires_with_the_window() {
        let mut state = RouteState::default();
        let t0 = 10_000.0;
        for id in 0..20 {
            state.record_routed(&AlertKey::new("health.alert", Some(100 + id)), t0);
        }
        assert_eq!(state.routed_1h(t0 + 3599.0), 20, "한 시간 이내의 적재 계수가 유실됐다");
        assert_eq!(decide(&state, &key(), &ctx(t0 + 3599.0)), Verdict::Hold(HoldReason::HourlyCap),
            "시간당 20건 상한이 창 안에서 풀렸다");
        assert_eq!(state.routed_1h(t0 + 3601.0), 0, "창 밖 적재가 계속 계수된다");
        assert_eq!(decide(&state, &key(), &ctx(t0 + 3601.0)), Verdict::Route,
            "한 시간 밖으로 이동한 뒤에도 상한 보류가 남았다");
    }

    // 입력 폭풍을 한 키로 병합하되 관측 횟수와 최초/최종 시각은 잃지 않는 핀이다.
    #[test]
    fn repeated_holds_merge_without_losing_counts() {
        let mut state = RouteState::default();
        for n in 0..100 {
            assert_eq!(state.record_hold(&key(), &format!("관측 {n}"), HoldReason::NoCso,
                10_000.0 + n as f64), None, "동일 키 병합에서 불필요한 접기가 발생했다");
        }
        assert_eq!(state.pending.len(), 1, "동일 키 보류가 여러 항목으로 늘어났다");
        let pending = state.pending.get(&key()).expect("병합한 보류 키가 사라졌다");
        assert_eq!(pending.count, 100, "병합 중 관측 횟수가 유실됐다");
        assert_eq!(pending.first_seen, 10_000.0, "병합이 최초 관측 시각을 덮어썼다");
        assert_eq!(pending.last_seen, 10_099.0, "병합이 최종 관측 시각을 갱신하지 않았다");
        assert_eq!(pending.summary, "관측 99", "최신 요약이 보류에 반영되지 않았다");
        assert_eq!(pending.reason, "no_cso", "보류 사유 문자열이 계약과 달라졌다");
        assert_eq!(state.suppressed_1h(10_099.0), 100, "병합 때문에 억제 계수가 줄었다");
    }

    // 가장 오래된 키의 원본 항목은 접혀도 관측 사실은 overflow에 남아야 한다.
    // 키 정렬 순서와 시간 순서를 다르게 만들어 실제로 가장 오래된 항목을 고르는지 지킨다.
    #[test]
    fn overflow_preserves_the_oldest_keys_observations() {
        let mut state = RouteState::default();
        let oldest = AlertKey::new("watchdog.zz_oldest", Some(9000));
        for n in 0..7 {
            assert_eq!(state.record_hold(&oldest, "오래된 경보", HoldReason::NoCso, 10_000.0 + n as f64),
                None, "상한 전의 반복 보류가 접혔다");
        }
        for n in 0..PENDING_MAX - 1 {
            let next = AlertKey::new("watchdog.aa_newer", Some(n as u64));
            assert_eq!(state.record_hold(&next, "새 경보", HoldReason::NoCso, 10_010.0 + n as f64),
                None, "키 상한에 도달하기 전에 접기가 발생했다");
        }
        let before = state.pending.values().map(|p| p.count).sum::<u64>();
        let added = AlertKey::new("watchdog.latest", Some(9999));
        let folded = state.record_hold(&added, "마지막 경보", HoldReason::NoCso, 11_000.0);
        assert_eq!(folded, Some(oldest.clone()), "접기 반환값이 가장 오래된 키가 아니다");
        assert!(!state.pending.contains_key(&oldest), "접힌 원본 키가 남아 중복 계수될 수 있다");
        let overflow = state.pending.get(&AlertKey::new(OVERFLOW_NAME, None))
            .expect("오래된 경보가 overflow 없이 사라졌다");
        assert_eq!(overflow.count, 7, "접힌 경보의 누적 횟수가 overflow에 보존되지 않았다");
        assert!(state.pending.contains_key(&added), "새로 추가한 경보가 접기에 휘말려 사라졌다");
        assert_eq!(state.pending.values().map(|p| p.count).sum::<u64>(), before + 1,
            "접기 전후 전체 관측 횟수가 보존되지 않았다");
    }

    // 적재 성공은 해당 보류만 해소하고 다음 동일 키 입력에는 쿨다운을 적용해야 한다.
    #[test]
    fn routing_removes_pending_and_starts_cooldown() {
        let mut state = RouteState::default();
        let other = AlertKey::new("health.alert", Some(9));
        state.record_hold(&key(), "보류", HoldReason::NoCso, 10_000.0);
        state.record_hold(&other, "다른 보류", HoldReason::NoCso, 10_000.0);
        state.record_routed(&key(), 10_001.0);
        assert!(!state.pending.contains_key(&key()), "적재한 키가 보류에 남았다");
        assert!(state.pending.contains_key(&other), "다른 키의 보류까지 삭제됐다");
        assert_eq!(state.last_routed.get(&key()), Some(&10_001.0), "적재 시각이 쿨다운에 기록되지 않았다");
        assert_eq!(state.routed_1h(10_002.0), 1, "적재 성공이 시간당 창에 기록되지 않았다");
        assert_eq!(decide(&state, &key(), &ctx(10_002.0)), Verdict::Hold(HoldReason::Cooldown),
            "적재 직후 동일 키가 쿨다운을 우회했다");
    }

    // 화면 제어문자와 연속 공백이 한 줄 문안을 오염시키지 못하게 한다.
    #[test]
    fn sanitize_removes_controls_and_collapses_spaces() {
        assert_eq!(sanitize_line("  가\n\t나\x1b  다\r\n ", 200), "가 나 다",
            "개행·탭·ESC 제거 또는 연속 공백 압축이 깨졌다");
    }

    // 눈에 보이지 않는 ZWSP가 정제된 기계 문안에 남는 회귀를 검출한다.
    #[test]
    fn sanitize_removes_zero_width_space() {
        let cleaned = sanitize_line("가\u{200b}나", 200);
        assert!(!cleaned.contains('\u{200b}'), "정제 결과에 ZWSP가 남았다");
        assert_eq!(cleaned.replace(' ', ""), "가나", "ZWSP 처리 중 실제 문자가 유실됐다");
    }

    // 바이트 상한이 한글 중간을 자르거나 상한보다 긴 UTF-8 결과를 내지 못하게 한다.
    #[test]
    fn sanitize_truncates_at_utf8_boundaries() {
        for (max, expected) in [(0, ""), (1, ""), (2, ""), (3, "가"),
            (4, "가"), (5, "가"), (6, "가나"), (7, "가나"), (9, "가나다")] {
            let cleaned = sanitize_line("가나다", max);
            assert!(cleaned.len() <= max, "정제 결과가 {max}바이트 상한을 넘었다");
            assert!(std::str::from_utf8(cleaned.as_bytes()).is_ok(), "절단 결과의 UTF-8이 손상됐다");
            assert_eq!(cleaned, expected, "{max}바이트 절단이 문자 경계를 지키지 않았다");
        }
    }

    // 알려진 다섯 이벤트의 필드명과 고정 서식이 바뀌어 사실 요약이 깨지는 것을 막는다.
    #[test]
    fn known_payloads_have_exact_formats() {
        let cases = [
            ("health.alert", json!({"rule": "cpu_high"}), "rule=cpu_high"),
            ("surface.exited", json!({"role": "worker", "agent": "codex"}), "role=worker agent=codex"),
            ("context.threshold", json!({"role": "worker", "context_pct": 75, "threshold": 60}),
                "role=worker context=75% threshold=60%"),
            ("queue.depth_high", json!({"depth": 51, "threshold": 50, "blocked_by": "paused"}),
                "depth=51/50 blocked_by=paused"),
            ("queue.starved", json!({"depth": 3, "head_wait_secs": 120, "blocked_by": "busy"}),
                "depth=3 head_wait=120s blocked_by=busy"),
        ];
        for (name, payload, expected) in cases {
            assert_eq!(summarize_payload(name, &payload), expected, "{name} 고정 요약 서식이 깨졌다");
        }
    }

    // 스칼라만 정렬해 네 개까지 싣고 자유 문장 금지 키가 요약에 섞이지 않도록 한다.
    #[test]
    fn generic_payload_sorts_limits_and_excludes_denied_keys() {
        let payload = json!({"z": 9, "d": "last", "c": true, "b": 2, "a": "first",
            "aa_array": [1], "ab_object": {"x": 1}, "ac_null": null,
            "line": "원문", "hint": "원문", "note": "원문", "action": "원문",
            "text": "원문", "message": "원문", "preview": "원문"});
        assert_eq!(summarize_payload("watchdog.load_high", &payload), "a=first b=2 c=true d=last",
            "일반 요약의 스칼라 선택·정렬·4개 상한이 깨졌다");
        let denied = json!({"line": "원문", "hint": "원문", "note": "원문", "action": "원문",
            "text": "원문", "message": "원문", "preview": "원문", "z": 9});
        assert_eq!(summarize_payload("watchdog.load_high", &denied), "z=9",
            "금지 키가 일반 요약에 포함됐다");
    }

    // 고정 및 일반 요약 모두 최종 정제와 200바이트 절단을 거치도록 지킨다.
    #[test]
    fn payload_summaries_are_sanitized_and_bounded() {
        assert_eq!(summarize_payload("health.alert", &json!({"rule": "가\n\t나\x1b  다"})),
            "rule=가 나 다", "고정 요약에서 최종 정제가 빠졌다");
        assert_eq!(summarize_payload("watchdog.load_high", &json!({"a": "가\n\t나\x1b  다"})),
            "a=가 나 다", "일반 요약에서 최종 정제가 빠졌다");
        for (name, field, prefix) in [("health.alert", "rule", "rule="), ("watchdog.load_high", "a", "a=")] {
            let mut payload = json!({});
            payload[field] = json!("한".repeat(100));
            let summary = summarize_payload(name, &payload);
            let expected = format!("{}{}", prefix, "한".repeat((200 - prefix.len()) / 3));
            assert_eq!(summary, expected, "{name}의 200바이트 문자 경계 절단이 깨졌다");
            assert!(summary.len() <= 200, "{name} 요약이 200바이트를 넘었다");
        }
    }

    // 봉투에서 비대상은 제외하고 좌석 결측은 임의 좌석으로 바꾸지 않는 핀이다.
    #[test]
    fn summarize_filters_names_and_preserves_missing_surface() {
        assert!(summarize(&json!({"name": "queue.enqueued", "payload": {}})).is_none(),
            "비대상 봉투가 경보로 변환됐다");
        let item = summarize(&json!({"name": "health.alert", "payload": {"rule": "cpu"}}))
            .expect("좌석 없는 대상 경보가 사라졌다");
        assert_eq!(item.key.surface, None, "결측 좌석이 임의 좌석으로 변환됐다");
        assert_eq!(item.key.name, "health.alert", "봉투의 경보 이름이 변조됐다");
        assert_eq!(item.summary, "rule=cpu", "봉투 payload 요약이 깨졌다");
    }

    // 좌석 결측 표기와 반복 문구 및 선두 기계 라벨을 정확한 문자열로 고정한다.
    #[test]
    fn render_text_keeps_exact_machine_label_and_repeat_format() {
        for (surface, repeat, expected) in [
            (Some(8), 1, "[alert] health.alert surface:8 rule=cpu"),
            (None, 1, "[alert] health.alert surface:- rule=cpu"),
            (Some(8), 2, "[alert] health.alert surface:8 rule=cpu (반복 2건)"),
            (None, 100, "[alert] health.alert surface:- rule=cpu (반복 100건)"),
        ] {
            let item = AlertItem { key: AlertKey::new("health.alert", surface), summary: "rule=cpu".into() };
            let text = render_text(&item, repeat);
            assert!(text.starts_with("[alert] "), "경보의 선두 기계 라벨이 깨졌다");
            assert_eq!(text, expected, "좌석 또는 반복 횟수의 문안 서식이 깨졌다");
        }
    }

    // 상태 JSON이 내부 누계 등을 누출하지 않고 계약의 네 키와 실제 계수만 내보내게 한다.
    #[test]
    fn snapshot_has_exactly_four_contract_keys() {
        let mut state = RouteState::default();
        state.enabled = true;
        state.record_routed(&key(), 10_000.0);
        state.record_hold(&AlertKey::new("health.alert", None), "보류", HoldReason::NoCso, 10_000.0);
        let snapshot = state.snapshot(10_001.0);
        let object = snapshot.as_object().expect("상태 스냅샷이 JSON 객체가 아니다");
        assert_eq!(object.len(), 4, "상태 스냅샷의 계약 키 개수가 네 개가 아니다");
        for name in ["enabled", "routed_1h", "suppressed_1h", "pending"] {
            assert!(object.contains_key(name), "상태 스냅샷에서 {name} 키가 빠졌다");
        }
        assert_eq!(snapshot, json!({"enabled": true, "routed_1h": 1, "suppressed_1h": 1, "pending": 1}),
            "상태 스냅샷의 계수 또는 활성 상태가 실제와 다르다");
    }

    // 환경변수는 공백·대소문자를 정규화한 명시적 거짓에만 꺼지도록 고정한다.
    #[test]
    fn enabled_from_accepts_only_explicit_false_values() {
        assert!(enabled_from(None), "미설정 기본값이 비활성으로 바뀌었다");
        for raw in ["0", "false", "off", "no", " OFF "] {
            assert!(!enabled_from(Some(raw)), "명시적 비활성 값 {raw:?}이 무시됐다");
        }
        for raw in ["1", "yes", ""] {
            assert!(enabled_from(Some(raw)), "활성 기본값 {raw:?}이 비활성으로 바뀌었다");
        }
    }

    // 한 분에 집중된 천 건도 정확히 세고 완전히 창 밖인 버킷은 읽기와 추가 후 모두 제외한다.
    // 분 버킷 해상도를 고려해 최초 분으로부터 61분 뒤에서 만료를 확인한다.
    #[test]
    fn minute_window_counts_bursts_and_expires_old_buckets() {
        let mut window = MinuteWindow::default();
        let t0 = 6000.0;
        assert_eq!(window.count(t0), 0, "빈 분 창의 계수가 0이 아니다");
        for _ in 0..1000 { window.add(t0 + 1.0); }
        assert_eq!(window.count(t0 + 59.0), 1000, "같은 초에 추가한 천 건의 계수가 틀렸다");
        assert_eq!(window.count(t0 + 3660.0), 0, "한 시간 밖의 분 버킷이 계수에 남았다");
        window.add(t0 + 3660.0);
        assert_eq!(window.count(t0 + 3660.0), 1, "만료 후 새 버킷에 과거 계수가 섞였다");
    }
}
