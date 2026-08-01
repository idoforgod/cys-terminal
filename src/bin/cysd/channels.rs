//! C0 채널 계층 코어 — Slack·Discord 브리지를 cysd(신뢰 경계)가 스폰·재조정하고,
//! 인바운드는 inbox-first 내구 퍼널로, 아웃바운드는 단조 상태기계(at-least-once)로 다룬다.
//! 정본 설계: `_research/openclaw-absorption/DESIGN-ko.md` §2 (2.0~2.8).
//!
//! 원리: **프로세스가 아니라 상태가 영속한다**(불사조와 동일). 브리지는 cysd 스폰 자식이고,
//! 채널 상태(desired-state·inbox·원장)는 channels.db가 소유한다. cysd/master가 정기 재시작되는
//! 시스템 전제 위에서 "내구 상태 + 부팅 재조정(reconcile)"으로 설계한다(§1.1).
//!
//! analytics.rs(analytics.db)와 별개 DB. rusqlite(이미 의존)·sha2(이미 의존)·libc(이미 의존).
//! open 실패는 graceful — Daemon.channels=None이면 채널 RPC는 channels_disabled를 돌려주고
//! 데몬은 계속 동작한다(순수 추가 계층·제거 가능). 무결이 필요한 곳(dedupe·단조 전이)은 트랜잭션.

use crate::handlers::Reply;
use crate::state::{Daemon, LedgerEntry, ProcessHealth};
use cys::{err_response, ok_response};
use rusqlite::{params, Connection, OptionalExtension};
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, OnceLock};

// ── 튜닝 상수 (테스트 주입 가능하게 분리 · §2.8) ──────────────────────────────
/// 아웃바운드 receipt 부재 시 unknown 전이 창(초). 기본 30s(§2.3). 테스트는 env로 축소 가능.
fn outbound_unknown_secs() -> f64 {
    std::env::var("CYS_CHANNEL_OUTBOUND_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30.0)
}
/// 종결 원장 행 보존기간(일·M5) — 이 기간 지난 terminal outbound·acked inbox·denied/suppressed
/// inbound·late_receipt를 주기 sweep이 프룬한다(무한성장·disk DoS 차단). 기본 7일·env override.
fn channel_retain_secs() -> f64 {
    std::env::var("CYS_CHANNEL_RETAIN_DAYS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| *d > 0.0)
        .unwrap_or(7.0)
        * 86400.0
}
/// 봇루프 슬라이딩 윈도우(초)와 상한 — 참여자쌍 20건/60초(+쿨다운 60초는 윈도우 유지로 발현).
const LOOP_WINDOW_SECS: f64 = 60.0;
const LOOP_LIMIT: u64 = 20;
/// inbox un-acked 재배달 TTL(초) — injected 후 10분 미-ack면 재주입(§2.2).
const INBOX_REDELIVER_TTL_SECS: f64 = 600.0;
/// 브리지 사망 후 재스폰 백오프(초) — 크래시 루프 폭주 차단.
const RESPAWN_BACKOFF_SECS: f64 = 5.0;
/// 연속 재스폰 실패 임계(M12) — 초과 시 health.alert 1회 발행·status에 down 표시(무한 재시도는 유지).
const RESPAWN_FAIL_ALERT_THRESHOLD: u64 = 5;
/// 주기 sweep 간격(초) — 재배달·타임아웃·브리지 사망 재조정.
const SWEEP_INTERVAL_SECS: u64 = 15;
/// 봉투 sender 표기 최대 길이(초과 시 … 절단).
const SENDER_MAXLEN: usize = 16;
/// 승인 미러 본문 요지 최대 길이(초과 시 … 절단·L4 매직넘버 상수화).
const SUMMARY_MAXLEN: usize = 200;
/// V7 일일 원격 승인(allow) 상한(§5) — 채널계정 탈취 시 무단 allow 폭발반경을 하루 N건으로 제한.
/// 초과분은 원격 거부·로컬 feed로 강등(fail-closed). deny 해소는 안전측이라 비계수(§4 오너 확정=20).
const REMOTE_APPROVE_DAILY_CAP: u64 = 20;
/// V4 연속 실패 임계(§5) — 한 sender가 interaction 검증에 연속 N회 실패하면 쿨다운(브루트 속도제한).
const SENDER_COOLDOWN_FAIL_THRESHOLD: u64 = 5;
/// V4 sender 쿨다운 지속(초) — 임계 도달 시 이 기간 원격 interaction 거부(로컬 feed는 불변).
/// nonce 불위조성이 1차 방어이고 쿨다운은 속도제한 보조. 15분=정당 owner 락아웃 최소·브루트 억제 충분.
const SENDER_COOLDOWN_SECS: f64 = 900.0;

// ── DB open + 스키마 ─────────────────────────────────────────────────────────

/// channels.db 열고 스키마 보장. 실패 시 None(graceful degrade — 채널 모듈 비활성).
pub fn open(socket_path: &Path) -> Option<Connection> {
    let path = crate::state::state_dir(socket_path).join("channels.db");
    let conn = Connection::open(&path).ok()?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
         -- desired-state 채널: enabled·스폰 pgid·토큰 해시(영속 핀)·등록 여부.
         CREATE TABLE IF NOT EXISTS channels(
            channel      TEXT PRIMARY KEY,
            enabled      INTEGER NOT NULL DEFAULT 0,
            bridge_cmd   TEXT,
            scoped_pid   INTEGER,
            scoped_pgid  INTEGER,
            token_hash   TEXT,
            registered   INTEGER NOT NULL DEFAULT 0,
            bridge_ver   TEXT,
            caps         TEXT,
            last_spawn_ts REAL,
            last_in_ts   REAL,
            last_out_ts  REAL,
            updated_ts   REAL NOT NULL);
         -- 아웃바운드 원장 + 단조 outcome(pending→terminal). approval 3필드·retry_of 체인.
         CREATE TABLE IF NOT EXISTS outbound(
            id            INTEGER PRIMARY KEY,
            channel       TEXT NOT NULL,
            target        TEXT NOT NULL,
            kind          TEXT NOT NULL,
            body          TEXT NOT NULL,
            reply_to      TEXT,
            idempotency_key TEXT NOT NULL,
            retry_of      INTEGER,
            outcome       TEXT NOT NULL,
            platform_ref  TEXT,
            detail        TEXT,
            approval_feed_id TEXT,
            approval_nonce   TEXT,
            approval_owner   TEXT,
            approval_nonce_used INTEGER NOT NULL DEFAULT 0,
            created_ts    REAL NOT NULL,
            updated_ts    REAL NOT NULL,
            UNIQUE(channel, idempotency_key));
         CREATE INDEX IF NOT EXISTS ix_outbound_pending ON outbound(outcome, created_ts);
         -- terminal 후 도착한 늦은 receipt. LOW-4: 관측·감사 레코드로 남긴다(상태 불변 —
         -- 능동 재전송 억제는 미구현. at-least-once 계약상 중복 허용, 상위 계층이 수동 참조).
         CREATE TABLE IF NOT EXISTS late_receipt(
            id           INTEGER PRIMARY KEY,
            outbound_id  INTEGER NOT NULL,
            outcome      TEXT NOT NULL,
            platform_ref TEXT,
            detail       TEXT,
            ts           REAL NOT NULL);
         -- 인바운드 원장 + dedupe(UNIQUE key·body_hash). 본문 비저장(프라이버시-바이-디자인·O7).
         CREATE TABLE IF NOT EXISTS inbound(
            id           INTEGER PRIMARY KEY,
            channel      TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            body_hash    TEXT NOT NULL,
            sender_id    TEXT,
            sender_kind  TEXT,
            peer         TEXT,
            msg_ref      TEXT,
            ts           REAL NOT NULL,
            verdict      TEXT NOT NULL,
            UNIQUE(channel, idempotency_key));
         -- 내구 inbox: 단조 id·state(new|injected|acked). text는 배달까지만 보관(ack 시 소거).
         CREATE TABLE IF NOT EXISTS inbox(
            id           INTEGER PRIMARY KEY,
            channel      TEXT NOT NULL,
            sender_id    TEXT,
            text         TEXT NOT NULL,
            state        TEXT NOT NULL,
            redelivered  INTEGER NOT NULL DEFAULT 0,
            created_ts   REAL NOT NULL,
            injected_ts  REAL,
            acked_ts     REAL);
         CREATE INDEX IF NOT EXISTS ix_inbox_state ON inbox(state, id);
         -- owner-only allowlist(fail-closed — 비면 전량 deny).
         CREATE TABLE IF NOT EXISTS allowlist(
            channel   TEXT NOT NULL,
            sender_id TEXT NOT NULL,
            added_ts  REAL NOT NULL,
            PRIMARY KEY(channel, sender_id));
         -- 봇루프 슬라이딩 윈도우 원장(참여자쌍 타임스탬프).
         CREATE TABLE IF NOT EXISTS loopwin(
            id        INTEGER PRIMARY KEY,
            channel   TEXT NOT NULL,
            sender_id TEXT NOT NULL,
            ts        REAL NOT NULL);
         CREATE INDEX IF NOT EXISTS ix_loopwin ON loopwin(channel, sender_id, ts);
         -- V4 원격 승인 브루트 방어: (채널,sender)별 연속 검증실패 카운터·쿨다운 만료 ts.
         -- 성공 해소가 카운터를 0으로 리셋한다(연속성 판정). 쿨다운은 시간창 — 계정 탈취 시
         -- 무단 시도의 속도를 제한한다(nonce 불위조성이 1차 방어·이건 보조 속도제한).
         CREATE TABLE IF NOT EXISTS approval_attempts(
            channel          TEXT NOT NULL,
            sender_id        TEXT NOT NULL,
            consecutive_fails INTEGER NOT NULL DEFAULT 0,
            cooldown_until   REAL NOT NULL DEFAULT 0,
            updated_ts       REAL NOT NULL DEFAULT 0,
            PRIMARY KEY(channel, sender_id));
         -- V7 일일 원격 allow 상한: 로컬 날짜(YYYY-MM-DD)별 성공 allow 해소 건수. 상한 초과 시
         -- 원격 allow 거부·로컬 강등(탈취 폭발반경 제한). deny 해소는 안전측이라 비계수.
         CREATE TABLE IF NOT EXISTS remote_approve_daily(
            day         TEXT PRIMARY KEY,
            allow_count INTEGER NOT NULL DEFAULT 0,
            updated_ts  REAL NOT NULL DEFAULT 0);",
    )
    .ok()?;
    // schema_version 핀(D8 마이그레이션 토대). 신규 DB는 3으로 출발(v3=OPP-21 원격승인 상한·쿨다운).
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('schema_version','3') ON CONFLICT(key) DO NOTHING",
        [],
    )
    .ok()?;
    // C2 마이그레이션: C0/C1이 만든 기존 channels.db(v1)에는 approval_nonce_used 컬럼이 없다.
    // ALTER는 컬럼이 이미 있으면 에러(신규 DB) → `let _`로 흡수해 멱등(신규·기존 모두 안전).
    // 승인 미러 nonce 단회 소각의 영속 플래그로, 재생(replay) 공격을 재시작 후에도 차단한다.
    let _ = conn.execute(
        "ALTER TABLE outbound ADD COLUMN approval_nonce_used INTEGER NOT NULL DEFAULT 0",
        [],
    );
    // v1 DB를 v2로 올린다(신규 DB는 이미 3 → no-op). 실패해도 graceful.
    let _ = conn.execute("UPDATE meta SET value='2' WHERE key='schema_version' AND value='1'", []);
    // OPP-21 v2→v3: approval_attempts·remote_approve_daily 테이블은 위 배치의 CREATE TABLE IF NOT
    // EXISTS가 신규·기존 DB 모두에 멱등 생성한다(추가전용·비파괴). 버전 핀만 올린다(그래도 graceful).
    let _ = conn.execute("UPDATE meta SET value='3' WHERE key='schema_version' AND value='2'", []);
    Some(conn)
}

// ── 순수 헬퍼(테스트 핀 가능) ────────────────────────────────────────────────

fn now() -> f64 {
    crate::state::now_epoch()
}

/// sha256 hex(토큰 핀·body_hash 산식 — pack.rs content_hash와 동형).
fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 1회용 32바이트 토큰의 hex 문자열(env CYS_CHANNEL_TOKEN로 브리지에 주입).
/// L2: unix=/dev/urandom·windows=CNG BCryptGenRandom. **실패 시 예측가능 폴백(sha256(pid,now))
/// 금지 — hard-fail(Err)** 로 예측가능 토큰/nonce를 원천 차단한다(브리지 인가·승인 nonce의 무결성
/// 근거). R-CLI-1: windows 시드 폴백을 실 CSPRNG(BCryptGenRandom·시스템 선호 RNG)로 대체.
/// ★GUI 오퍼레이터 승인(오너 2026-07-15): 오퍼레이터 토큰 발급(state.rs Daemon::new)이 재사용 —
/// pub(crate) 승격(동일 CSPRNG 정책 공유·중복 구현 금지).
pub(crate) fn random_token_hex() -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom")
            .map_err(|e| format!("open /dev/urandom failed: {e}"))?;
        let mut buf = [0u8; 32];
        f.read_exact(&mut buf)
            .map_err(|e| format!("read /dev/urandom failed: {e}"))?;
        Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(windows)]
    {
        // R-CLI-1: Windows CNG BCryptGenRandom(시스템 선호 CSPRNG)로 실 난수 확보. 예측가능
        // 시드 폴백(sha256(pid,now,SystemTime)) 제거 — 브리지 토큰·승인 nonce 예측불가성 확보.
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        let mut buf = [0u8; 32];
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        // BCryptGenRandom은 성공 시 STATUS_SUCCESS(0) — 비0은 hard-fail(예측가능 폴백 금지).
        if status != 0 {
            return Err(format!("BCryptGenRandom 실패: NTSTATUS {status:#010x}"));
        }
        Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// 봇루프 판정(순수) — 윈도우 내 카운트가 상한 초과면 억제.
fn loop_suppressed(count_in_window: u64, limit: u64) -> bool {
    count_in_window > limit
}

/// 아웃바운드 receipt가 '늦은' 것인가(순수) — pending이 아니면 이미 terminal이라 late.
fn receipt_is_late(current_outcome: &str) -> bool {
    current_outcome != "pending"
}

/// tier가 채널 미러 가능한가(순수·fail-closed·§2.4-3 S8). None(무태그)·"d"·미지값은 **절대 미러 금지**
/// — Tier D(비가역·외부발행·헌법변경)는 구조적으로 채널에 도달 불가. "a"|"b"|"c"만 true.
fn tier_mirrorable(tier: Option<&str>) -> bool {
    matches!(tier, Some("a") | Some("b") | Some("c"))
}

/// V6 redact 필터(순수·§5 V6) — 아웃바운드 카드에 실릴 title·요지에서 **토큰·개인경로**를 스크럽한다.
/// 채널 계정 탈취 시에도 승인 카드 본문으로 비밀이 새지 않게 원문 전문 발송을 금지하고(원문은 로컬
/// feed 포인터), 매칭 스팬만 `[redacted]`로 치환한다. 반환 = (스크럽본, redact 발생 여부).
/// 패턴(deny-by-shape): Slack/GitHub/OpenAI/AWS 토큰 접두·장문 opaque(hex 32+·base64류 40+)·
/// bearer·홈 경로(/Users/·/home/·C:\Users\). 오탐(정상 문장 스크럽)보다 누출이 위험하므로
/// fail-closed 측(넓게 잡음)이되 일반 단어·짧은 토큰은 보존한다(길이 하한으로 구분).
fn redact(s: &str) -> (String, bool) {
    static PATS: OnceLock<Vec<Regex>> = OnceLock::new();
    let pats = PATS.get_or_init(|| {
        [
            // Slack: xoxb-/xoxp-/xoxa-/xoxr-/xoxs- 등 + 하이픈 세그먼트.
            r"xox[baprs]-[A-Za-z0-9-]{8,}",
            // OpenAI sk-, GitHub ghp_/gho_/ghs_/ghr_/github_pat_, Google AIza.
            r"sk-[A-Za-z0-9_-]{16,}",
            r"gh[pousr]_[A-Za-z0-9]{20,}",
            r"github_pat_[A-Za-z0-9_]{20,}",
            r"AIza[A-Za-z0-9_-]{20,}",
            // AWS access key id.
            r"AKIA[0-9A-Z]{16}",
            // Bearer 토큰.
            r"(?i)bearer\s+[A-Za-z0-9._-]{16,}",
            // JWT(3 base64url 세그먼트).
            r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
            // 홈/사용자 경로(개인정보) — 뒤 세그먼트까지 삼킨다.
            r"/Users/[^/\s]+(?:/[^\s]*)?",
            r"/home/[^/\s]+(?:/[^\s]*)?",
            r"[Cc]:\\Users\\[^\\\s]+(?:\\[^\s]*)?",
            // 장문 opaque hex(32+) — 시크릿/해시 유출 방어(짧은 hex는 보존).
            r"\b[0-9a-fA-F]{32,}\b",
            // 장문 opaque base64url류(40+, 하이픈/언더스코어 포함) — 일반 단어(공백 분절)와 구분.
            r"\b[A-Za-z0-9_-]{40,}\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    });
    let mut out = s.to_string();
    let mut hit = false;
    for re in pats {
        if re.is_match(&out) {
            hit = true;
            out = re.replace_all(&out, "[redacted]").into_owned();
        }
    }
    (out, hit)
}

/// V7 일일 상한 키(순수) — epoch 초를 로컬 날짜 `YYYY-MM-DD` 문자열로. 상한은 로컬 하루 단위.
fn local_date(ts: f64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".into())
}

/// V4 쿨다운 활성 판정(순수) — 만료 ts가 현재보다 미래면 쿨다운 중.
fn cooldown_active(cooldown_until: f64, now: f64) -> bool {
    cooldown_until > now
}

/// V7 상한 도달 판정(순수) — 오늘 allow 건수가 상한 이상이면 추가 원격 allow 금지(fail-closed).
fn cap_reached(count: u64, cap: u64) -> bool {
    count >= cap
}

/// inbox 재배달 대상 판정(순수·테스트 핀) — new는 즉시, injected는 TTL 초과 un-acked면 재주입.
/// 실경로(deliver_new_inbox·redeliver_unacked)는 동일 규칙을 SQL WHERE로 집행한다(등가 계약 박제).
#[allow(dead_code)] // 계약 문서화 순수 술어 — SQL 집행과 동형(state.rs is_reusable 선례).
fn redeliver_due(state: &str, injected_ts: Option<f64>, now: f64, ttl: f64) -> bool {
    match state {
        "new" => true,
        "injected" => injected_ts.map(|t| now - t >= ttl).unwrap_or(false),
        _ => false,
    }
}

/// 채널 인바운드 미신뢰 텍스트 살균(C1·순수) — 봉투 조립 전 ESC·C0 제어·CR·DEL을 제거한다.
/// bracketed-paste 종료 시퀀스(`\x1b[201~`)나 CR(`\r`)이 채널 text에 섞이면, master(무승인) PTY에
/// 붙여넣기가 조기 종료되어 이후 바이트가 실제 키입력·조기 제출되는 경계 붕괴가 난다(state.rs 주입은
/// `\x1b[200~{text}\x1b[201~`+`\r`). 여기서 미신뢰 text의 위험 바이트를 봉투 진입 전에 없앤다.
/// 보존: HT(`\t`)·LF(`\n`)·인쇄 가능 문자(한글·이모지 등 U+0020 이상, DEL 제외). 제거: 그 외 C0(0x00-0x1F
/// 중 \t·\n 제외 — CR·ESC 포함)·DEL(0x7F)·C1(0x80-0x9F, 8비트 CSI `\u{9b}`·NEL `\u{85}` 포함)·
/// U+2028(LS)·U+2029(PS). 8비트 C1을 CSI로 해석하는 수신 터미널에서 bracketed-paste 조기 종료를 막는다.
/// 화이트리스트가 아니라 범위검사면 C1·LS/PS가 새어 키입력 주입이 되므로 명시 제외한다(MED-1 감사).
/// 채널 전용 살균 — 공유 WriteReq::Inject·cys send 신뢰 경로는 절대 건드리지 않는다(회귀 방지·§C1).
fn sanitize_inbound_text(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            c == '\n'
                || c == '\t'
                || (c >= ' '
                    && c != '\u{7f}'
                    && !('\u{80}'..='\u{9f}').contains(&c)
                    && c != '\u{2028}'
                    && c != '\u{2029}')
        })
        .collect()
}

/// 봉투 문자열: `[CH:<channel>|<sender>|<HH:MM>|#<inbox_id>] <text>` (+재배달 표기).
fn envelope(channel: &str, sender: &str, ts: f64, inbox_id: i64, redelivered: bool, text: &str) -> String {
    // C1: 채널에서 온 미신뢰 text·channel·sender를 봉투에 넣기 직전 동일 불변식으로 살균한다
    // (ESC/C0/C1/LS/PS 유입 0). text만 살균하고 channel·sender는 미살균이면 같은 봉투에
    // 제어시퀀스가 보간되는 경계 붕괴가 난다(LOW-1 감사). sender는 char 단위 truncate 전에 살균한다.
    let text = sanitize_inbound_text(text);
    let channel = sanitize_inbound_text(channel);
    let sender = sanitize_inbound_text(sender);
    let short: String = if sender.chars().count() > SENDER_MAXLEN {
        let head: String = sender.chars().take(SENDER_MAXLEN).collect();
        format!("{head}…")
    } else {
        sender
    };
    let hhmm = {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_opt(ts as i64, 0)
            .single()
            .map(|dt| dt.format("%H:%M").to_string())
            .unwrap_or_else(|| "--:--".into())
    };
    let mark = if redelivered { " (재배달)" } else { "" };
    format!("[CH:{channel}|{short}|{hhmm}|#{inbox_id}]{mark} {text}")
}

fn p_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}
fn p_i64(params: &Value, key: &str) -> Option<i64> {
    params
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
}
fn p_f64(params: &Value, key: &str) -> Option<f64> {
    params
        .get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
}

/// pid가 속한 프로세스 그룹(unix). 브리지 스폰 그룹 소속 검증(§2.1-3 pid 핀)에 쓴다.
#[cfg(unix)]
fn pgid_of(pid: u32) -> Option<i32> {
    let r = unsafe { libc::getpgid(pid as libc::pid_t) };
    if r < 0 {
        None
    } else {
        Some(r as i32)
    }
}
#[cfg(windows)]
fn pgid_of(_pid: u32) -> Option<i32> {
    None // Windows pid-핀은 C1 WINFIX에서 대칭화 — C0은 토큰 단독(문서화).
}

/// pid 생존 프로브(unix=kill(pid,0)). windows는 best-effort(C1 WINFIX 전까지).
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    // L3 한계 명문화: windows는 pid 생존 프로브가 없어 **항상 alive로 보고**한다 — 그 결과
    // ①respawn_dead_bridges의 자가치유(죽은 브리지 재스폰)가 발현 안 하고(dead 판정 불가)
    // ②channel.status의 alive가 항상 true다. 재스폰 감지는 reaper 스레드 신호(on_bridge_exit)에만
    // 의존한다. 실제 프로브(OpenProcess/GetExitCodeProcess) 편입은 WINFIX 트랙. doctor도 이 한계를
    // 경고한다(diag_channels_db).
    pid != 0
}

// ── 브리지 스폰(cysd 스폰 자식·scoped 원장 등록·reaper 스레드로 zombie 회수) ────

/// bridge_cmd를 새 프로세스 그룹에서 스폰하고 (pid, pgid) 반환. env로 1회용 토큰·채널·소켓 주입.
/// scoped=true로 daemon 원장에 등록(shutdown_cleanup·reap_orphan_ledger 계약 재사용).
/// reaper 스레드가 wait()로 zombie를 회수하고 종료 시 registered=0 마킹+bridge.exited 발행한다.
fn spawn_bridge(
    daemon: &Arc<Daemon>,
    channel: &str,
    bridge_cmd: &str,
    token: &str,
) -> Result<(u32, i32), String> {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(bridge_cmd);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(bridge_cmd);
        c
    };
    cmd.env("CYS_CHANNEL_TOKEN", token)
        .env("CYS_CHANNEL", channel)
        .env(cys::ENV_SOCKET, daemon.socket_path.to_string_lossy().as_ref());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid(); // 새 세션/그룹 → pgid == pid, 그룹 단위 회수 가능.
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        // CREATE_NO_WINDOW 동시 지정 — creation_flags 는 덮어쓰기라 hide_console() 별도 호출과
        // 병용 불가. 없으면 콘솔 없는 cysd가 띄우는 장수 브리지(cmd /C)마다 콘솔 창이 뜬다.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let child = cmd.spawn().map_err(|e| format!("bridge spawn failed: {e}"))?;
    let pid = child.id();
    let pgid = pid as i32; // setsid → pgid == pid(unix); windows는 pid 사용.

    // scoped 원장 등록 — surface_id=None(소유 surface 없음). reap_orphan_ledger는 surface_id
    // Some일 때만 '소유 surface 소멸' kill을 하므로 None은 pid 사망 시에만 원장에서 제거된다.
    // shutdown_cleanup(collect_scoped_for_shutdown)은 scoped 전량을 pgid로 회수 → 데몬 종료 시
    // 브리지 트리 동반 종료(§2.1-6).
    daemon.ledger.lock().unwrap().insert(
        pid,
        LedgerEntry {
            pid,
            pgid,
            cmd: format!("channel-bridge:{channel}"),
            surface_id: None,
            scoped: true,
            registered_at: now(),
            caps: None,
            health: ProcessHealth::Reusable,
        },
    );

    // reaper 스레드: 블로킹 wait()로 zombie 회수 + 종료 신호. std 스레드라 tokio 런타임 비의존
    // (RPC 동기 경로·sweep 동기 경로 어디서 스폰해도 안전).
    let d = Arc::clone(daemon);
    let ch = channel.to_string();
    let mut child = child;
    std::thread::spawn(move || {
        let _ = child.wait();
        on_bridge_exit(&d, &ch, pid);
    });
    Ok((pid, pgid))
}

/// 브리지 종료 콜백 — 현재 채널 scoped_pid가 이 pid일 때만(=새 스폰이 교체하지 않았을 때만)
/// registered=0 마킹 + channel.bridge.exited 발행. 원장 항목도 제거. 재스폰은 sweep 정책 소관.
fn on_bridge_exit(daemon: &Arc<Daemon>, channel: &str, pid: u32) {
    {
        let guard = daemon.channels.lock().unwrap();
        if let Some(conn) = guard.as_ref() {
            let cur: Option<i64> = conn
                .query_row(
                    "SELECT scoped_pid FROM channels WHERE channel=?1",
                    [channel],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten();
            if cur != Some(pid as i64) {
                return; // 이미 재스폰이 교체함 — 이 종료는 구 프로세스라 무시.
            }
            let _ = conn.execute(
                "UPDATE channels SET registered=0, updated_ts=?2 WHERE channel=?1",
                params![channel, now()],
            );
        } else {
            return;
        }
    }
    daemon.ledger.lock().unwrap().remove(&pid);
    daemon.bus.publish(
        "channel.bridge.exited",
        "channel",
        None,
        json!({"channel": channel, "pid": pid}),
    );
}

// ── RPC 위임 ─────────────────────────────────────────────────────────────────

/// dispatch에서 `channel.<sub>` 전량을 여기로 위임한다(match 비대화 방지·모듈 자기완결).
pub fn handle(daemon: &Arc<Daemon>, sub: &str, params: &Value, id: &Value, caller_pid: Option<u32>) -> Reply {
    let mut guard = daemon.channels.lock().unwrap();
    let Some(conn) = guard.as_mut() else {
        return Reply::Single(err_response(
            id,
            "channels_disabled",
            "channels module unavailable (channels.db open failed)",
        ));
    };
    let resp: Value = match sub {
        "start" => start(daemon, conn, params, id),
        "stop" => stop(daemon, conn, params, id),
        "register" => register(daemon, conn, params, id, caller_pid),
        "inbound" => inbound(daemon, conn, params, id, caller_pid),
        "outbound" => outbound(daemon, conn, params, id),
        "receipt" => receipt(daemon, conn, params, id),
        "ack" => ack(conn, params, id),
        "allow" => allow(conn, params, id),
        "allow-remote-approve" => allow_remote_approve(conn, params, id),
        "revoke" => revoke(conn, params, id),
        "lockdown" => lockdown(daemon, conn, id),
        "unlock" => unlock(daemon, conn, id),
        "status" => status(conn, id),
        other => err_response(id, "unknown_method", &format!("unknown channel method: channel.{other}")),
    };
    Reply::Single(resp)
}

fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}
fn meta_set(conn: &Connection, key: &str, val: &str) {
    let _ = conn.execute(
        "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
        params![key, val],
    );
}
fn lockdown_active(conn: &Connection) -> bool {
    meta_get(conn, "lockdown").as_deref() == Some("1")
}

/// Tier C 원격 승인 opt-in 만료 절대 ts(초). 미설정/파싱실패=0(=상시 만료·기본 OFF·§2.4-5).
fn remote_approve_until(conn: &Connection) -> f64 {
    meta_get(conn, "allow_remote_approve_until")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}
/// 원격 승인 게이트가 지금 열려있는가 — 만료 ts가 now보다 미래일 때만. 기본 OFF(폰 분실 시 안전측).
/// 이 게이트가 닫혀있으면 **미러 발행도, interaction 승인 처리도** 하지 않는다(꺼짐=버튼 없음).
fn remote_approve_active(conn: &Connection, now: f64) -> bool {
    remote_approve_until(conn) > now
}

// ── channel.start / stop ─────────────────────────────────────────────────────

fn start(daemon: &Arc<Daemon>, conn: &mut Connection, params: &Value, id: &Value) -> Value {
    // H3: 채널은 메인 cysd 단독 소유(§2.5) — 부서 데몬은 브리지 스폰 금지(동일 봇토큰 이중
    // 게이트웨이 연결 차단). 부서 데몬은 자기 channels.db만 검사해 교차 스폰을 못 막으므로,
    // 스폰 진입점(start·register)에서 구조적으로 거부한다.
    if cys::is_dept_socket(&daemon.socket_path) {
        return err_response(
            id,
            "dept_channel_forbidden",
            "채널은 메인 cysd 단독 소유 — 부서 데몬은 브리지 스폰 불가",
        );
    }
    let Some(channel) = p_str(params, "channel") else {
        return err_response(id, "invalid_params", "missing channel");
    };
    // 동일 채널 활성 레코드 존재 시 거부(§2.1-4 이중 연결 금지).
    let existing: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT enabled, scoped_pid FROM channels WHERE channel=?1",
            [&channel],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    if let Some((1, Some(pid))) = existing {
        if pid_alive(pid as u32) {
            return err_response(
                id,
                "channel_active",
                &format!("channel '{channel}' already active (pid {pid}) — stop first"),
            );
        }
    }
    // bridge_cmd: --cmd로 신규 등록하거나 기록된 것 재사용. 둘 다 없으면 스폰 불가.
    let bridge_cmd = p_str(params, "cmd").or_else(|| {
        conn.query_row("SELECT bridge_cmd FROM channels WHERE channel=?1", [&channel], |r| r.get::<_, Option<String>>(0))
            .optional()
            .ok()
            .flatten()
            .flatten()
    });
    let Some(bridge_cmd) = bridge_cmd else {
        return err_response(id, "invalid_params", "no bridge command (pass --cmd on first start)");
    };
    let token = match random_token_hex() {
        Ok(t) => t,
        Err(e) => return err_response(id, "token_gen_failed", &e), // L2: 예측가능 폴백 금지.
    };
    let token_hash = hex_sha256(token.as_bytes());
    let (pid, pgid) = match spawn_bridge(daemon, &channel, &bridge_cmd, &token) {
        Ok(v) => v,
        Err(e) => return err_response(id, "spawn_failed", &e),
    };
    let ts = now();
    let r = conn.execute(
        "INSERT INTO channels(channel, enabled, bridge_cmd, scoped_pid, scoped_pgid, token_hash, registered, last_spawn_ts, updated_ts)
         VALUES(?1,1,?2,?3,?4,?5,0,?6,?6)
         ON CONFLICT(channel) DO UPDATE SET
            enabled=1, bridge_cmd=?2, scoped_pid=?3, scoped_pgid=?4, token_hash=?5, registered=0,
            last_spawn_ts=?6, updated_ts=?6",
        params![channel, bridge_cmd, pid as i64, pgid as i64, token_hash, ts],
    );
    if let Err(e) = r {
        // DB 반영 실패 = 브리지 생명주기를 추적 불가 → 즉시 회수(고아 방지).
        crate::governance::kill_group_or_pid(pid, pgid);
        daemon.ledger.lock().unwrap().remove(&pid);
        return err_response(id, "db_error", &format!("persist failed, bridge killed: {e}"));
    }
    // start가 명시 owner 동작이므로 이 채널의 lockdown 잔재는 없다고 보되, 전역 lockdown은
    // 명시 재개 신호로 해제하지 않는다(안전측 — lockdown 해제는 별도 owner 결정).
    ok_response(id, json!({"channel": channel, "pid": pid, "pgid": pgid, "started": true}))
}

fn stop(daemon: &Arc<Daemon>, conn: &mut Connection, params: &Value, id: &Value) -> Value {
    let Some(channel) = p_str(params, "channel") else {
        return err_response(id, "invalid_params", "missing channel");
    };
    let row: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT scoped_pid, scoped_pgid FROM channels WHERE channel=?1",
            [&channel],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    if let Some((Some(pid), pgid)) = row {
        crate::governance::kill_group_or_pid(pid as u32, pgid.unwrap_or(pid) as i32);
        daemon.ledger.lock().unwrap().remove(&(pid as u32));
    }
    let _ = conn.execute(
        "UPDATE channels SET enabled=0, registered=0, scoped_pid=NULL, scoped_pgid=NULL, updated_ts=?2 WHERE channel=?1",
        params![channel, now()],
    );
    ok_response(id, json!({"channel": channel, "stopped": true}))
}

// ── channel.register ─────────────────────────────────────────────────────────

fn register(daemon: &Arc<Daemon>, conn: &mut Connection, params: &Value, id: &Value, caller_pid: Option<u32>) -> Value {
    // H3: 부서 데몬은 브리지를 스폰하지 않으므로 등록도 받지 않는다(메인 단독 소유·§2.5).
    if cys::is_dept_socket(&daemon.socket_path) {
        return err_response(
            id,
            "dept_channel_forbidden",
            "채널은 메인 cysd 단독 소유 — 부서 데몬은 브리지 등록 불가",
        );
    }
    let Some(channel) = p_str(params, "channel") else {
        return err_response(id, "invalid_params", "missing channel");
    };
    let Some(token) = p_str(params, "token") else {
        return err_response(id, "invalid_params", "missing token");
    };
    let row: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT token_hash, scoped_pgid FROM channels WHERE channel=?1 AND enabled=1",
            [&channel],
            |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let Some((token_hash, scoped_pgid)) = row else {
        return err_response(id, "not_started", &format!("channel '{channel}' is not started"));
    };
    // ① 토큰 해시 일치(§2.1-3).
    if hex_sha256(token.as_bytes()) != token_hash {
        daemon.bus.publish(
            "channel.auth.denied",
            "channel",
            None,
            json!({"channel": channel, "reason": "token mismatch", "caller_pid": caller_pid}),
        );
        return err_response(id, "auth_denied", "token mismatch");
    }
    // ② caller_pid가 스폰 pgid 소속(pid 핀). unix 전용 — windows는 토큰 단독(C1 WINFIX).
    #[cfg(unix)]
    {
        let ok = caller_pid
            .and_then(pgid_of)
            .zip(scoped_pgid)
            .map(|(cg, sg)| cg as i64 == sg)
            .unwrap_or(false);
        if !ok {
            daemon.bus.publish(
                "channel.auth.denied",
                "channel",
                None,
                json!({"channel": channel, "reason": "caller pid not in spawn process group",
                       "caller_pid": caller_pid, "scoped_pgid": scoped_pgid}),
            );
            return err_response(id, "auth_denied", "caller pid not in bridge spawn process group");
        }
    }
    #[cfg(windows)]
    let _ = scoped_pgid;
    let bridge_ver = p_str(params, "bridge_ver");
    let caps = p_str(params, "caps");
    let _ = conn.execute(
        "UPDATE channels SET registered=1, bridge_ver=?2, caps=?3, updated_ts=?4 WHERE channel=?1",
        params![channel, bridge_ver, caps, now()],
    );
    // §2.1-5 S1: 응답에 pending outbound 전량 동봉(재스폰 갭 중 발행분 재조정·outbound_id dedupe).
    let mut pending = pending_outbound(conn, &channel);
    // M6: sent 됐지만 feed가 아직 pending인 approval_prompt도 재조정(승인버튼 복원). outbound_id dedupe.
    let seen: std::collections::HashSet<i64> = pending
        .iter()
        .filter_map(|v| v.get("outbound_id").and_then(|x| x.as_i64()))
        .collect();
    pending.extend(sent_pending_approvals(daemon, conn, &channel, &seen));
    daemon.bus.publish(
        "channel.registered",
        "channel",
        None,
        json!({"channel": channel, "bridge_ver": bridge_ver, "pending": pending.len()}),
    );
    ok_response(id, json!({"channel": channel, "registered": true, "pending": pending}))
}

fn pending_outbound(conn: &Connection, channel: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, target, kind, body, reply_to, idempotency_key, retry_of,
                approval_feed_id, approval_nonce, approval_owner
         FROM outbound WHERE channel=?1 AND outcome='pending' ORDER BY id",
    ) {
        if let Ok(rows) = stmt.query_map([channel], |r| {
            Ok(json!({
                "outbound_id": r.get::<_, i64>(0)?,
                "target": r.get::<_, String>(1)?,
                "kind": r.get::<_, String>(2)?,
                "body": r.get::<_, String>(3)?,
                "reply_to": r.get::<_, Option<String>>(4)?,
                "idempotency_key": r.get::<_, String>(5)?,
                "retry_of": r.get::<_, Option<i64>>(6)?,
                "approval": approval_json(
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ),
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}

fn approval_json(feed_id: Option<String>, nonce: Option<String>, owner: Option<String>) -> Value {
    if feed_id.is_none() && nonce.is_none() && owner.is_none() {
        Value::Null
    } else {
        json!({"feed_id": feed_id, "nonce": nonce, "owner_sender_id": owner})
    }
}

/// M6: 이미 sent(비-pending)됐지만 대상 feed가 아직 pending인 approval_prompt를 register 재조정에
/// 포함시킨다(브리지 재스폰 후 승인버튼 복원). pending_outbound는 outcome='pending'만 잡아 sent 버튼을
/// 놓치는데, 그 사이 feed가 미해소면 새 브리지엔 버튼이 안 뜨고 승인이 영영 원격으로 안 온다.
/// nonce 미소각(재사용 가능) + feed pending인 행만 재-emit. outbound_id는 seen으로 dedupe.
fn sent_pending_approvals(
    daemon: &Arc<Daemon>,
    conn: &Connection,
    channel: &str,
    seen: &std::collections::HashSet<i64>,
) -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, target, kind, body, reply_to, idempotency_key, retry_of,
                approval_feed_id, approval_nonce, approval_owner
         FROM outbound
         WHERE channel=?1 AND kind='approval_prompt' AND outcome!='pending' AND approval_nonce_used=0
         ORDER BY id",
    ) else {
        return out;
    };
    let rows = stmt.query_map([channel], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    });
    let Ok(rows) = rows else {
        return out;
    };
    for (oid, target, kind, body, reply_to, key, retry_of, feed, nonce, owner) in rows.flatten() {
        if seen.contains(&oid) {
            continue; // pending_outbound에 이미 포함(중복 방지).
        }
        // feed가 여전히 pending일 때만 버튼 복원(해소된 것은 재렌더 불필요).
        let feed_pending = feed
            .as_deref()
            .map(|f| daemon.feed_item_pending(f))
            .unwrap_or(false);
        if !feed_pending {
            continue;
        }
        out.push(json!({
            "outbound_id": oid, "target": target, "kind": kind, "body": body,
            "reply_to": reply_to, "idempotency_key": key, "retry_of": retry_of,
            "approval": approval_json(feed, nonce, owner),
        }));
    }
    out
}

// ── 승인 미러(§2.4·§2.6 O9) — feed.push/aging 시 tier≤C·게이트 ON이면 채널 버튼 발행 ──────

/// 미러 버튼 본문: 제목·요지(절단)·feed_id·유효기간. nonce는 표시 텍스트가 아니라 outbound
/// approval_* 원장·이벤트 payload(버튼 custom_id)로만 흐른다(§2.3 각인).
fn mirror_body(title: &str, summary: &str, feed_id: &str, until: f64) -> String {
    // V6: title·요지를 카드 조립 전 redact 필터 경유(토큰·개인경로 스크럽). redact 후 절단해
    // 창 안에 남은 비밀도 스크럽되게 한다(절단 후 redact면 창 경계의 비밀이 샐 수 있음).
    let (title_r, hit_t) = redact(title);
    let (summary_r, hit_s) = redact(summary);
    let redacted = hit_t || hit_s;
    let s: String = summary_r.chars().take(SUMMARY_MAXLEN).collect();
    let ell = if summary_r.chars().count() > SUMMARY_MAXLEN { "…" } else { "" };
    let title = title_r.as_str();
    // redact 발생 시 원문은 로컬 feed 전용임을 명시(§5 V6: 원문=로컬 패널 포인터).
    let note = if redacted { "\n(민감정보 가림 — 전문은 로컬 cys 패널 확인)" } else { "" };
    let hhmm = {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_opt(until as i64, 0)
            .single()
            .map(|dt| dt.format("%H:%M").to_string())
            .unwrap_or_else(|| "--:--".into())
    };
    format!("[승인 요청] {title}\n{s}{ell}\nfeed:{feed_id}\n(원격 승인 버튼 · 유효 ~{hhmm}){note}")
}

/// 한 (채널,owner)에 approval_prompt 미러 1건을 멱등 발행한다(§2.6 O9: feed당 1버튼).
/// idempotency_key=`approval:<feed_id>:<owner>` — 재호출(aging)은 기존 행 존재로 skip(중복 버튼 0).
/// 신규 발행 시 32B hex nonce 단회를 outbound 원장에 각인하고 channel.outbound.<ch> 이벤트를 낸다
/// (pause 중이면 행만 남기고 이벤트 동결 — resume_flush가 재발행·outbound() 정책과 동형).
fn mirror_one(daemon: &Arc<Daemon>, conn: &Connection, channel: &str, owner: &str, feed_id: &str, body: &str) {
    let key = format!("approval:{feed_id}:{owner}");
    let exists = conn
        .query_row(
            "SELECT 1 FROM outbound WHERE channel=?1 AND idempotency_key=?2",
            params![channel, key],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if exists {
        return; // 이미 미러됨 — 중복 버튼 금지(aging은 갱신이지 신규 발행 아님).
    }
    // L2: nonce 생성 실패 시 예측가능 폴백 금지 — 미러를 건너뛴다(약한 nonce 버튼 발행 방지).
    let Ok(nonce) = random_token_hex() else {
        return;
    };
    let ts = now();
    if conn
        .execute(
            "INSERT INTO outbound(channel, target, kind, body, reply_to, idempotency_key, retry_of, outcome,
                                  approval_feed_id, approval_nonce, approval_owner, approval_nonce_used, created_ts, updated_ts)
             VALUES(?1,?2,'approval_prompt',?3,NULL,?4,NULL,'pending',?5,?6,?2,0,?7,?7)",
            params![channel, owner, body, key, feed_id, nonce, ts],
        )
        .is_err()
    {
        return;
    }
    let oid = conn.last_insert_rowid();
    let _ = conn.execute("UPDATE channels SET last_out_ts=?2 WHERE channel=?1", params![channel, ts]);
    if !daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
        daemon.bus.publish(
            &format!("channel.outbound.{channel}"),
            "channel",
            None,
            json!({"outbound_id": oid, "channel": channel, "target": owner, "kind": "approval_prompt",
                   "body": body, "reply_to": Value::Null, "idempotency_key": key, "retry_of": Value::Null,
                   "approval": {"feed_id": feed_id, "nonce": nonce, "owner_sender_id": owner}}),
        );
    }
}

/// 승인 미러 진입점 — feed.push·aging에서 호출. **fail-closed 2중 게이트**:
/// ① tier≤C(a|b|c)만(무태그/D 차단) ② 원격승인 게이트 ON(OFF면 버튼 없음이 안전측). 둘 다 통과 시
/// enabled+registered 채널의 owner allowlist마다 멱등 미러(§2.4·§2.6 O9). 데몬 lock: channels만
/// 잡는다(호출자는 feed_items 락을 이미 해제 — lock-order 안전).
pub fn mirror_approval(daemon: &Arc<Daemon>, feed_id: &str, title: &str, summary: &str, tier: Option<&str>) {
    if !tier_mirrorable(tier) {
        return; // ① tier fail-closed — 무태그/D는 절대 미러 금지.
    }
    let mut guard = daemon.channels.lock().unwrap();
    let Some(conn) = guard.as_mut() else {
        return;
    };
    if !remote_approve_active(conn, now()) {
        return; // ② 원격승인 게이트 OFF — 미러 발행 자체 금지.
    }
    let until = remote_approve_until(conn);
    // (채널, owner) 쌍을 먼저 수집(prepared stmt 대여 종료) 후 삽입 — borrow 충돌 회피.
    let channels: Vec<String> = conn
        .prepare("SELECT channel FROM channels WHERE enabled=1 AND registered=1")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    let mut targets: Vec<(String, String)> = Vec::new();
    for ch in &channels {
        let owners: Vec<String> = conn
            .prepare("SELECT sender_id FROM allowlist WHERE channel=?1")
            .and_then(|mut s| s.query_map([ch], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>())
            .unwrap_or_default();
        for o in owners {
            targets.push((ch.clone(), o));
        }
    }
    let body = mirror_body(title, summary, feed_id, until);
    for (ch, owner) in targets {
        mirror_one(daemon, conn, &ch, &owner, feed_id, &body);
    }
}

// ── channel.inbound — inbox-first 내구 퍼널(§2.2 ①~⑤) ─────────────────────────

fn inbound(daemon: &Arc<Daemon>, conn: &mut Connection, params: &Value, id: &Value, caller_pid: Option<u32>) -> Value {
    let Some(channel) = p_str(params, "channel") else {
        return err_response(id, "invalid_params", "missing channel");
    };
    let Some(idempotency_key) = p_str(params, "idempotency_key") else {
        return err_response(id, "invalid_params", "missing idempotency_key");
    };
    let text = p_str(params, "text").unwrap_or_default();
    let sender_id = p_str(params, "sender_id").unwrap_or_default();
    let sender_kind = p_str(params, "sender_kind").unwrap_or_else(|| "user".into());
    let peer = p_str(params, "peer");
    let msg_ref = p_str(params, "msg_ref");
    let ts = p_f64(params, "ts").unwrap_or_else(now);
    // body_hash: 브리지 제공값 우선, 없으면 text로 산출(계약 명세 — S3).
    let body_hash = p_str(params, "body_hash").unwrap_or_else(|| hex_sha256(text.as_bytes()));

    // lockdown 중이면 인바운드 전면 차단(§2.4-5).
    if lockdown_active(conn) {
        return err_response(id, "locked", "channel layer is in lockdown");
    }

    // ── ① 인가: 채널 enabled+registered + caller_pid가 스폰 pgid 소속(pid 핀) ──
    let auth: Option<(i64, i64, Option<i64>)> = conn
        .query_row(
            "SELECT enabled, registered, scoped_pgid FROM channels WHERE channel=?1",
            [&channel],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .ok()
        .flatten();
    let Some((enabled, registered, scoped_pgid)) = auth else {
        return err_response(id, "not_registered", &format!("channel '{channel}' unknown"));
    };
    if enabled != 1 || registered != 1 {
        return err_response(id, "not_registered", &format!("channel '{channel}' not registered/enabled"));
    }
    #[cfg(unix)]
    {
        let ok = caller_pid
            .and_then(pgid_of)
            .zip(scoped_pgid)
            .map(|(cg, sg)| cg as i64 == sg)
            .unwrap_or(false);
        if !ok {
            daemon.bus.publish(
                "channel.auth.denied",
                "channel",
                None,
                json!({"channel": channel, "reason": "inbound caller not in bridge group",
                       "caller_pid": caller_pid}),
            );
            return err_response(id, "auth_denied", "inbound caller pid not in bridge spawn process group");
        }
    }
    #[cfg(windows)]
    let _ = (caller_pid, scoped_pgid);

    // ── 승인 버튼 클릭(interaction) 분기(§2.4-4·§2.7) — 브리지 인가를 통과한 뒤에만.
    // kind=interaction이면 inbox 퍼널이 아니라 nonce 3중 검증 경로로 간다(원격 승인 해소).
    if p_str(params, "kind").as_deref() == Some("interaction") {
        return verify_interaction(daemon, conn, &channel, params, id);
    }

    // 이하 판정·적재는 단일 트랜잭션(dedupe check+insert 원자성).
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(e) => return err_response(id, "db_error", &e.to_string()),
    };

    // ── ② dedupe: 같은 (channel, key) 존재 시 body_hash 비교 ──
    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT body_hash, verdict FROM inbound WHERE channel=?1 AND idempotency_key=?2",
            params![channel, idempotency_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    if let Some((prior_hash, prior_verdict)) = prior {
        if prior_hash == body_hash {
            let _ = tx.commit();
            return ok_response(id, json!({"action": "dup", "verdict": prior_verdict}));
        } else {
            let _ = tx.commit();
            // 같은 키·다른 해시 = 조용한 드롭 금지, 경보(S3·O11).
            daemon.bus.publish(
                "channel.dedup.conflict",
                "channel",
                None,
                json!({"channel": channel, "idempotency_key": idempotency_key,
                       "prior_hash": prior_hash, "new_hash": body_hash}),
            );
            return err_response(id, "dedup_conflict", "same idempotency_key with different body_hash");
        }
    }

    // ── ③ owner-only fail-closed: allowlist(channel, sender_id) 존재해야 통과 ──
    // M9: owner-only를 봇루프보다 **앞**에 둔다 — 비owner sender는 loopwin 행조차 만들지 못하게
    // 즉시 deny(자원 절약·원장 무한성장 완화). 봇루프는 인가된 sender에만 적용된다.
    let allowed: bool = tx
        .query_row(
            "SELECT 1 FROM allowlist WHERE channel=?1 AND sender_id=?2",
            params![channel, sender_id],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if !allowed {
        let _ = tx.execute(
            "INSERT INTO inbound(channel, idempotency_key, body_hash, sender_id, sender_kind, peer, msg_ref, ts, verdict)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'denied')",
            params![channel, idempotency_key, body_hash, sender_id, sender_kind, peer, msg_ref, ts],
        );
        let _ = tx.execute(
            "UPDATE channels SET last_in_ts=?2 WHERE channel=?1",
            params![channel, ts],
        );
        let _ = tx.commit();
        daemon.bus.publish(
            "channel.auth.denied",
            "channel",
            None,
            json!({"channel": channel, "sender_id": sender_id, "reason": "not in owner allowlist"}),
        );
        return ok_response(id, json!({"action": "deny", "reason": "sender not in owner allowlist"}));
    }

    // ── ④ 봇루프: (인가된) sender_kind=bot이면 20건/60s 슬라이딩 윈도우 ──
    if sender_kind == "bot" {
        let _ = tx.execute(
            "INSERT INTO loopwin(channel, sender_id, ts) VALUES(?1,?2,?3)",
            params![channel, sender_id, ts],
        );
        let cnt: u64 = tx
            .query_row(
                "SELECT COUNT(*) FROM loopwin WHERE channel=?1 AND sender_id=?2 AND ts >= ?3",
                params![channel, sender_id, ts - LOOP_WINDOW_SECS],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if loop_suppressed(cnt, LOOP_LIMIT) {
            let _ = tx.execute(
                "INSERT INTO inbound(channel, idempotency_key, body_hash, sender_id, sender_kind, peer, msg_ref, ts, verdict)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'loop_suppressed')",
                params![channel, idempotency_key, body_hash, sender_id, sender_kind, peer, msg_ref, ts],
            );
            let _ = tx.commit();
            daemon.bus.publish(
                "channel.loop.suppressed",
                "channel",
                None,
                json!({"channel": channel, "sender_id": sender_id, "count": cnt, "limit": LOOP_LIMIT}),
            );
            return ok_response(id, json!({"action": "suppressed", "count": cnt}));
        }
    }

    // ── ⑤ inbox 적재(state=new, 단조 id) + 인바운드 원장(accepted) ──
    if tx
        .execute(
            "INSERT INTO inbox(channel, sender_id, text, state, created_ts) VALUES(?1,?2,?3,'new',?4)",
            params![channel, sender_id, text, ts],
        )
        .is_err()
    {
        return err_response(id, "db_error", "inbox insert failed");
    }
    let inbox_id = tx.last_insert_rowid();
    let _ = tx.execute(
        "INSERT INTO inbound(channel, idempotency_key, body_hash, sender_id, sender_kind, peer, msg_ref, ts, verdict)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'accepted')",
        params![channel, idempotency_key, body_hash, sender_id, sender_kind, peer, msg_ref, ts],
    );
    let _ = tx.execute(
        "UPDATE channels SET last_in_ts=?2 WHERE channel=?1",
        params![channel, ts],
    );
    if let Err(e) = tx.commit() {
        return err_response(id, "db_error", &e.to_string());
    }

    // 즉시 배달 시도(master 가용+비-quiescing이면 주입, 아니면 queued 유지).
    let delivered = deliver_new_inbox(daemon, conn);
    let action = if delivered.contains(&inbox_id) { "delivered" } else { "queued" };
    let evt = if action == "delivered" { "channel.message" } else { "channel.message.queued" };
    daemon.bus.publish(
        evt,
        "channel",
        None,
        json!({"channel": channel, "inbox_id": inbox_id, "sender_id": sender_id}),
    );
    ok_response(id, json!({"action": action, "inbox_id": inbox_id}))
}

// ── V4/V7 원격 승인 브루트·상한 원장 헬퍼(§5) ────────────────────────────────

/// V4: (채널,sender) 연속 검증실패 1 증가. 임계 도달 시 쿨다운 설정+카운터 리셋(만료 후 새 슬레이트).
/// 반환 = 이번 실패로 쿨다운이 발동됐는지(이벤트 표기용). 성공 해소는 reset_attempt로 0 복귀.
fn record_attempt_failure(conn: &Connection, channel: &str, sender_id: &str, now: f64) -> bool {
    let _ = conn.execute(
        "INSERT INTO approval_attempts(channel, sender_id, consecutive_fails, cooldown_until, updated_ts)
         VALUES(?1,?2,1,0,?3)
         ON CONFLICT(channel,sender_id) DO UPDATE SET consecutive_fails=consecutive_fails+1, updated_ts=?3",
        params![channel, sender_id, now],
    );
    let fails: u64 = conn
        .query_row(
            "SELECT consecutive_fails FROM approval_attempts WHERE channel=?1 AND sender_id=?2",
            params![channel, sender_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(0)
        .max(0) as u64;
    if fails >= SENDER_COOLDOWN_FAIL_THRESHOLD {
        let _ = conn.execute(
            "UPDATE approval_attempts SET consecutive_fails=0, cooldown_until=?3, updated_ts=?4
             WHERE channel=?1 AND sender_id=?2",
            params![channel, sender_id, now + SENDER_COOLDOWN_SECS, now],
        );
        return true;
    }
    false
}

/// V4: 성공 해소 시 (채널,sender) 연속실패·쿨다운을 0으로 리셋(연속성 판정 — 성공이 브루트 의심 해제).
fn reset_attempt(conn: &Connection, channel: &str, sender_id: &str, now: f64) {
    let _ = conn.execute(
        "UPDATE approval_attempts SET consecutive_fails=0, cooldown_until=0, updated_ts=?3
         WHERE channel=?1 AND sender_id=?2",
        params![channel, sender_id, now],
    );
}

/// V4: (채널,sender) 현재 쿨다운 만료 ts(없으면 0.0). cooldown_active와 함께 진입 게이트에서 쓴다.
fn attempt_cooldown_until(conn: &Connection, channel: &str, sender_id: &str) -> f64 {
    conn.query_row(
        "SELECT cooldown_until FROM approval_attempts WHERE channel=?1 AND sender_id=?2",
        params![channel, sender_id],
        |r| r.get::<_, f64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0.0)
}

/// V7: 로컬 날짜 day의 성공 원격 allow 건수(없으면 0). 상한 대조에 쓴다.
fn daily_allow_count(conn: &Connection, day: &str) -> u64 {
    conn.query_row(
        "SELECT allow_count FROM remote_approve_daily WHERE day=?1",
        params![day],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0)
    .max(0) as u64
}

/// V7: 로컬 날짜 day의 성공 원격 allow 건수 +1(성공 allow 해소 직후 호출).
fn bump_daily_allow(conn: &Connection, day: &str, now: f64) {
    let _ = conn.execute(
        "INSERT INTO remote_approve_daily(day, allow_count, updated_ts) VALUES(?1,1,?2)
         ON CONFLICT(day) DO UPDATE SET allow_count=allow_count+1, updated_ts=?2",
        params![day, now],
    );
}

// ── 승인 버튼 interaction 검증(§2.4-4) — nonce 3중 + 게이트 + allow-once ─────────

/// 브리지가 보고한 버튼 클릭을 검증해 원격 승인을 해소한다. 순서(각 실패는 구분된 사유+이벤트):
/// ★V4 쿨다운(연속 실패 sender 속도제한) ⓪ 게이트 ON(allow-remote-approve 유효) ① sender=owner
/// allowlist ② decision∈{allow,deny} ③ nonce 실재+feed 일치+미사용+owner 바인딩 ④ feed_id 실재·pending
/// ★V7 일일 allow 상한(초과 시 로컬 강등) ⑤ nonce 원자 소각(재생 차단) ⑥ feed reply 경로로 해소 →
/// channel.approval.resolved. allow는 allow-once만(단회 nonce가 강제).
/// 거부는 2종: **fail_deny**(위조·미인가 시도 = V4 연속실패 계수) vs **soft_deny**(정책·게이트·상한·레이스
/// = 정당 owner거나 무해 → 비계수, 정당 owner 락아웃 방지). 성공 해소는 카운터 리셋(+allow는 상한 +1).
fn verify_interaction(daemon: &Arc<Daemon>, conn: &Connection, channel: &str, params: &Value, id: &Value) -> Value {
    let sender_id = p_str(params, "sender_id").unwrap_or_default();
    let Some(feed_id) = p_str(params, "feed_id") else {
        return err_response(id, "invalid_params", "missing feed_id");
    };
    let Some(nonce) = p_str(params, "nonce") else {
        return err_response(id, "invalid_params", "missing nonce");
    };
    let decision = p_str(params, "decision").unwrap_or_default();
    let now_ts = now();

    // 거부 헬퍼 — 이벤트 발행 + interaction_denied 결과(브리지가 ephemeral 회신).
    let publish_denied = |reason: &str, cooldown_tripped: bool| {
        daemon.bus.publish(
            "channel.approval.denied",
            "channel",
            None,
            json!({"channel": channel, "feed_id": feed_id, "sender_id": sender_id,
                   "reason": reason, "cooldown_tripped": cooldown_tripped}),
        );
    };
    // fail_deny: 위조·미인가 시도 — V4 연속실패 카운터 증가(임계 도달 시 쿨다운 발동).
    let fail_deny = |reason: &str| -> Value {
        let tripped = record_attempt_failure(conn, channel, &sender_id, now_ts);
        publish_denied(reason, tripped);
        ok_response(id, json!({"action": "interaction_denied", "reason": reason}))
    };
    // soft_deny: 정책·게이트·상한·무해 레이스 — 비계수(정당 owner 락아웃·brute 오탐 방지).
    let soft_deny = |reason: &str| -> Value {
        publish_denied(reason, false);
        ok_response(id, json!({"action": "interaction_denied", "reason": reason}))
    };

    // ★V4 진입 게이트: sender가 쿨다운 중이면 즉시 비계수 거부(가장 저렴·게이트 상태 비노출).
    if cooldown_active(attempt_cooldown_until(conn, channel, &sender_id), now_ts) {
        return soft_deny("sender_cooldown");
    }
    // ⓪ 원격 승인 게이트(OFF면 버튼 무효 — 폰 분실 안전측). 비계수(owner 정책).
    if !remote_approve_active(conn, now_ts) {
        return soft_deny("remote_approve_off");
    }
    // ① sender=owner allowlist(fail-closed). 미인가 sender = fail 계수(V1·V4 브루트 주표적).
    let is_owner = conn
        .query_row(
            "SELECT 1 FROM allowlist WHERE channel=?1 AND sender_id=?2",
            params![channel, sender_id],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if !is_owner {
        return fail_deny("not_owner");
    }
    // ② decision 화이트리스트(allow|deny만 — allow-always 등 불가). 변조 = fail 계수.
    if decision != "allow" && decision != "deny" {
        return fail_deny("bad_decision");
    }
    // ③ nonce 실재 + feed 일치 + 미사용 + owner 바인딩(다른 owner의 nonce 도용 차단).
    let row: Option<(i64, String, i64, Option<String>)> = conn
        .query_row(
            "SELECT id, approval_feed_id, approval_nonce_used, approval_owner
             FROM outbound WHERE channel=?1 AND approval_nonce=?2 AND kind='approval_prompt'",
            params![channel, nonce],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten();
    let Some((oid, row_feed, used, owner)) = row else {
        return fail_deny("nonce_invalid"); // V4 브루트 주표적 — 축약 오타·비존재 nonce.
    };
    if row_feed != feed_id {
        return fail_deny("nonce_feed_mismatch");
    }
    if used != 0 {
        return soft_deny("nonce_used"); // 유효 nonce 재생·double-click — 창은 valid, brute 아님(비계수).
    }
    if owner.as_deref() != Some(sender_id.as_str()) {
        return fail_deny("owner_mismatch"); // 타 owner nonce 도용 = 미인가(계수).
    }
    // ④ feed_id 실재·pending(해소 대상이 살아있어야 함). M8: feed_items 직접 순회 대신 캡슐 헬퍼.
    if !daemon.feed_item_pending(&feed_id) {
        return soft_deny("feed_not_pending"); // 유효 creds·stale feed — brute 아님(비계수).
    }
    // ★V7 일일 원격 allow 상한 — nonce 소각 前 검사(초과 시 소각·해소 없이 pending 유지=로컬 강등).
    // allow만 계수·검사(deny 해소는 안전측이라 상한 비적용). soft_deny(정당 owner·비계수).
    if decision == "allow" && cap_reached(daily_allow_count(conn, &local_date(now_ts)), REMOTE_APPROVE_DAILY_CAP) {
        return soft_deny("daily_cap");
    }
    // ⑤ nonce 원자 소각(WHERE used=0 → 0행이면 동시 재생이 이미 태움 = 재생 차단).
    let burned = conn
        .execute(
            "UPDATE outbound SET approval_nonce_used=1, updated_ts=?2 WHERE id=?1 AND approval_nonce_used=0",
            params![oid, now_ts],
        )
        .unwrap_or(0);
    if burned == 0 {
        return soft_deny("nonce_race"); // 동시 소각 레이스 — 창은 valid였음(비계수).
    }
    // ⑥ 기존 feed reply 경로로 해소(pending→resolved·대기 pusher wake·feed.item.resolved).
    match daemon.resolve_feed_item(&feed_id, &decision) {
        Some(_) => {
            // 성공 = brute 의심 해제 → V4 카운터 리셋. allow면 V7 일일 상한 +1(소각~bump 사이 동시
            // allow 경합의 off-by-one 초과는 허용 — 상한은 soft 폭발반경 제한이라 정밀 원자성 불요).
            reset_attempt(conn, channel, &sender_id, now_ts);
            if decision == "allow" {
                bump_daily_allow(conn, &local_date(now_ts), now_ts);
            }
            daemon.bus.publish(
                "channel.approval.resolved",
                "channel",
                None,
                json!({"channel": channel, "feed_id": feed_id, "sender_id": sender_id, "decision": decision}),
            );
            ok_response(id, json!({"action": "approval_resolved", "feed_id": feed_id, "decision": decision}))
        }
        // 소각~해소 사이 레이스로 feed가 사라짐(다른 경로가 해소) — nonce는 이미 태워 안전(비계수).
        None => soft_deny("feed_gone"),
    }
}

// ── inbox 배달기(§2.2 배달 상태기계) ─────────────────────────────────────────

/// master surface가 (a) resolve_role 성공 (b) quiescing 아님 (c) 데몬 non-paused 이면 그
/// surface_id, 아니면 None.
fn deliverable_master(daemon: &Arc<Daemon>) -> Option<u64> {
    // pause(kill-switch) 게이트(§2.6 O5): pause 중엔 inbox 주입을 동결한다(적재·inbound 판정은
    // 계속=유실 0). outbound 이벤트 발행 동결과 짝이며, resume 시 resume_flush가 드레인한다.
    if daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let sid = {
        let roles = daemon.roles.lock().unwrap();
        roles.get("master").copied()?
    };
    let surface = daemon.get_surface(sid)?;
    if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    // quiescing 게이트(S5): 자기보고 상태가 quiescing이면 주입 보류.
    let quiescing = surface
        .agent_status
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.state == "quiescing")
        .unwrap_or(false);
    if quiescing {
        return None;
    }
    Some(sid)
}

/// master stdin에 봉투를 주입(bracketed paste + Return). schedule.rs inject와 동형.
/// MED-3: `try_send` **직전에** paused·exited·quiescing을 재확인한다. 루프 상단 게이트
/// (deliverable_master)는 최초 판정일 뿐이라, 매 주입 직전 재확인으로 mid-loop quiescing set
/// (cycle-agent가 /clear 진입 직전 quiescing을 set하는 창)을 봉합한다(잔여 나노초 창은 self-heal —
/// 배달 불가면 false 반환→호출부 break로 남은 행은 queued 유지). deliverable_master(daemon)==Some(sid)로
/// sid 일치까지 확인해 master surface가 루프 중 교체된 경우도 방어한다.
fn inject_master(daemon: &Arc<Daemon>, sid: u64, envelope: &str) -> bool {
    if deliverable_master(daemon) != Some(sid) {
        return false; // 주입 직전 재확인 실패(paused/exited/quiescing/surface 교체) → 보류(queued).
    }
    let Some(surface) = daemon.get_surface(sid) else {
        return false;
    };
    // ★R1 배달 원장 — 주입보다 앞(delivery.rs 불변식 ①). 외부 채널 봉투도 기계 유래다.
    crate::delivery::record_audited(
        daemon,
        sid,
        envelope,
        crate::delivery::Origin::Channel,
        None,
    );
    surface
        .write_tx
        .try_send(crate::state::WriteReq::Inject {
            text: envelope.to_string(),
            cr_delay_ms: 500,
            clear_first: false,
        })
        .is_ok()
}

/// state=new inbox 항목을 단조 id 순서로 배달(master 가용+비-quiescing일 때만). 배달된 inbox_id들 반환.
fn deliver_new_inbox(daemon: &Arc<Daemon>, conn: &Connection) -> Vec<i64> {
    let mut delivered = Vec::new();
    let Some(sid) = deliverable_master(daemon) else {
        return delivered; // master 부재/quiescing → 적재만(queued).
    };
    let rows: Vec<(i64, String, String, f64)> = {
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, channel, sender_id, created_ts FROM inbox WHERE state='new' ORDER BY id",
        ) else {
            return delivered;
        };
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, f64>(3)?,
            ))
        });
        match mapped {
            Ok(m) => m.flatten().collect(),
            Err(_) => return delivered,
        }
    };
    for (inbox_id, channel, sender_id, created_ts) in rows {
        let text: String = conn
            .query_row("SELECT text FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
            .unwrap_or_default();
        let env = envelope(&channel, &sender_id, created_ts, inbox_id, false, &text);
        if inject_master(daemon, sid, &env) {
            let _ = conn.execute(
                "UPDATE inbox SET state='injected', injected_ts=?2 WHERE id=?1",
                params![inbox_id, now()],
            );
            delivered.push(inbox_id);
        } else {
            break; // 주입 채널 정체 — 다음 sweep에서 재시도(순서 보존).
        }
    }
    delivered
}

/// un-acked 재배달 sweep(§2.2): injected 후 TTL 초과 미-ack 항목을 재주입(`(재배달)` 표기).
fn redeliver_unacked(daemon: &Arc<Daemon>, conn: &Connection) -> usize {
    let Some(sid) = deliverable_master(daemon) else {
        return 0;
    };
    let now = now();
    let rows: Vec<(i64, String, String, f64)> = conn
        .prepare(
            "SELECT id, channel, sender_id, created_ts FROM inbox
             WHERE state='injected' AND injected_ts IS NOT NULL AND (?1 - injected_ts) >= ?2 ORDER BY id",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![now, INBOX_REDELIVER_TTL_SECS], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    r.get::<_, f64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let mut n = 0;
    for (inbox_id, channel, sender_id, created_ts) in rows {
        let text: String = conn
            .query_row("SELECT text FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
            .unwrap_or_default();
        let env = envelope(&channel, &sender_id, created_ts, inbox_id, true, &text);
        if inject_master(daemon, sid, &env) {
            let _ = conn.execute(
                "UPDATE inbox SET injected_ts=?2, redelivered=1 WHERE id=?1",
                params![inbox_id, now],
            );
            n += 1;
        }
    }
    n
}

// ── channel.ack ──────────────────────────────────────────────────────────────

fn ack(conn: &mut Connection, params: &Value, id: &Value) -> Value {
    let Some(inbox_id) = p_i64(params, "inbox_id") else {
        return err_response(id, "invalid_params", "missing inbox_id");
    };
    // ack 시 text 소거(프라이버시-바이-디자인·O7 — 배달 완료 후 본문 비보관).
    let n = conn
        .execute(
            "UPDATE inbox SET state='acked', acked_ts=?2, text='' WHERE id=?1 AND state!='acked'",
            params![inbox_id, now()],
        )
        .unwrap_or(0);
    if n == 0 {
        return err_response(id, "not_found", &format!("no un-acked inbox item {inbox_id}"));
    }
    ok_response(id, json!({"inbox_id": inbox_id, "acked": true}))
}

// ── channel.outbound — 단조 상태기계 + at-least-once(§2.3) ────────────────────

fn outbound(daemon: &Arc<Daemon>, conn: &mut Connection, params: &Value, id: &Value) -> Value {
    // L1: lockdown 중엔 outbound도 게이트한다(행 생성·이벤트 발행 차단·완결성). inbound·reconcile와
    // 대칭 — 긴급 잠금은 발신도 멈춘다. 해제는 channel.unlock(H2).
    if lockdown_active(conn) {
        return err_response(id, "lockdown_active", "channel lockdown active — outbound blocked");
    }
    let Some(channel) = p_str(params, "channel") else {
        return err_response(id, "invalid_params", "missing channel");
    };
    let Some(target) = p_str(params, "target") else {
        return err_response(id, "invalid_params", "missing target");
    };
    let Some(idempotency_key) = p_str(params, "idempotency_key") else {
        return err_response(id, "invalid_params", "missing idempotency_key");
    };
    let kind = p_str(params, "kind").unwrap_or_else(|| "message".into());
    let body = p_str(params, "body").unwrap_or_default();
    let reply_to = p_str(params, "reply_to");
    let retry_of = p_i64(params, "retry_of");
    let (ap_feed, ap_nonce, ap_owner) = match params.get("approval") {
        Some(a) if a.is_object() => (
            a.get("feed_id").and_then(|v| v.as_str()).map(String::from),
            a.get("nonce").and_then(|v| v.as_str()).map(String::from),
            a.get("owner_sender_id").and_then(|v| v.as_str()).map(String::from),
        ),
        _ => (None, None, None),
    };

    // owner allowlist 대상만(fail-closed) — target이 allowlist에 없으면 발신 거부.
    let allowed: bool = conn
        .query_row(
            "SELECT 1 FROM allowlist WHERE channel=?1 AND sender_id=?2",
            params![channel, target],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if !allowed {
        return err_response(id, "target_not_allowed", "outbound target not in owner allowlist (fail-closed)");
    }

    // idempotency(§2.3): 같은 (channel, key) 재호출 = 기존 행 outcome 반환(신규 생성 안 함).
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, outcome FROM outbound WHERE channel=?1 AND idempotency_key=?2",
            params![channel, idempotency_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    if let Some((oid, outcome)) = existing {
        return ok_response(id, json!({"outbound_id": oid, "outcome": outcome, "idempotent": true}));
    }

    let ts = now();
    if conn
        .execute(
            "INSERT INTO outbound(channel, target, kind, body, reply_to, idempotency_key, retry_of, outcome,
                                  approval_feed_id, approval_nonce, approval_owner, created_ts, updated_ts)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9,?10,?11,?11)",
            params![channel, target, kind, body, reply_to, idempotency_key, retry_of,
                    ap_feed, ap_nonce, ap_owner, ts],
        )
        .is_err()
    {
        return err_response(id, "db_error", "outbound insert failed");
    }
    let oid = conn.last_insert_rowid();
    let _ = conn.execute(
        "UPDATE channels SET last_out_ts=?2 WHERE channel=?1",
        params![channel, ts],
    );
    // 브리지가 구독할 전문 이벤트(channel.outbound.<ch>) — payload에 outbound_id·전문.
    // pause(§2.6 O5) 중엔 발행을 동결한다(행은 pending 유지·유실 0). resume 시 resume_flush가
    // 미확정(pending) 아웃바운드를 재발행해 브리지가 배달 지시를 다시 받는다.
    if !daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
        daemon.bus.publish(
            &format!("channel.outbound.{channel}"),
            "channel",
            None,
            json!({"outbound_id": oid, "channel": channel, "target": target, "kind": kind,
                   "body": body, "reply_to": reply_to, "idempotency_key": idempotency_key,
                   "retry_of": retry_of, "approval": approval_json(ap_feed, ap_nonce, ap_owner)}),
        );
    }
    ok_response(id, json!({"outbound_id": oid, "outcome": "pending"}))
}

// ── channel.receipt — 단조 전이 + late_receipt 화해(§2.3) ─────────────────────

fn receipt(daemon: &Arc<Daemon>, conn: &mut Connection, params: &Value, id: &Value) -> Value {
    let Some(oid) = p_i64(params, "outbound_id") else {
        return err_response(id, "invalid_params", "missing outbound_id");
    };
    let Some(outcome) = p_str(params, "outcome") else {
        return err_response(id, "invalid_params", "missing outcome");
    };
    const TERMINAL: [&str; 5] = ["sent", "suppressed", "partial_failed", "failed", "unknown"];
    if !TERMINAL.contains(&outcome.as_str()) {
        return err_response(id, "invalid_params", &format!("outcome must be one of {TERMINAL:?}"));
    }
    let platform_ref = p_str(params, "platform_ref");
    let detail = p_str(params, "detail");
    let current: Option<String> = conn
        .query_row("SELECT outcome FROM outbound WHERE id=?1", [oid], |r| r.get(0))
        .optional()
        .ok()
        .flatten();
    let Some(current) = current else {
        return err_response(id, "not_found", &format!("no outbound {oid}"));
    };
    if receipt_is_late(&current) {
        // 이미 terminal — 상태 불변, late_receipt 관측·감사 레코드 + 이벤트(§2.3, 재전송 억제 미구현).
        let _ = conn.execute(
            "INSERT INTO late_receipt(outbound_id, outcome, platform_ref, detail, ts) VALUES(?1,?2,?3,?4,?5)",
            params![oid, outcome, platform_ref, detail, now()],
        );
        daemon.bus.publish(
            "channel.receipt.late",
            "channel",
            None,
            json!({"outbound_id": oid, "current": current, "late_outcome": outcome}),
        );
        return ok_response(id, json!({"outbound_id": oid, "outcome": current, "late": true}));
    }
    // pending → terminal(단조 1회).
    let _ = conn.execute(
        "UPDATE outbound SET outcome=?2, platform_ref=?3, detail=?4, updated_ts=?5 WHERE id=?1",
        params![oid, outcome, platform_ref, detail, now()],
    );
    if outcome == "failed" {
        daemon.bus.publish(
            "channel.receipt.failed",
            "channel",
            None,
            json!({"outbound_id": oid, "detail": detail}),
        );
    }
    ok_response(id, json!({"outbound_id": oid, "outcome": outcome, "late": false}))
}

// ── channel.allow / revoke ───────────────────────────────────────────────────

fn allow(conn: &mut Connection, params: &Value, id: &Value) -> Value {
    let (Some(channel), Some(sender_id)) = (p_str(params, "channel"), p_str(params, "sender_id")) else {
        return err_response(id, "invalid_params", "missing channel or sender_id");
    };
    let _ = conn.execute(
        "INSERT INTO allowlist(channel, sender_id, added_ts) VALUES(?1,?2,?3)
         ON CONFLICT(channel, sender_id) DO NOTHING",
        params![channel, sender_id, now()],
    );
    ok_response(id, json!({"channel": channel, "sender_id": sender_id, "allowed": true}))
}

// ── channel.allow-remote-approve — Tier C 원격 승인 기간 한정 opt-in(§2.4-5) ────

/// `cys channel allow-remote-approve --for <기간>` → meta에 만료 ts 기록(기본 OFF에서 기간 열기).
/// duration_secs=0(또는 --off)이면 즉시 닫는다(만료 ts=0=상시 만료). 폰 분실 시 기본이 안전측.
fn allow_remote_approve(conn: &mut Connection, params: &Value, id: &Value) -> Value {
    let secs = p_f64(params, "duration_secs").unwrap_or(0.0);
    if secs <= 0.0 {
        meta_set(conn, "allow_remote_approve_until", "0");
        return ok_response(id, json!({"allow_remote_approve": false, "until": 0.0}));
    }
    let until = now() + secs;
    meta_set(conn, "allow_remote_approve_until", &until.to_string());
    ok_response(id, json!({"allow_remote_approve": true, "until": until, "for_secs": secs}))
}

fn revoke(conn: &mut Connection, params: &Value, id: &Value) -> Value {
    let (Some(channel), Some(sender_id)) = (p_str(params, "channel"), p_str(params, "sender_id")) else {
        return err_response(id, "invalid_params", "missing channel or sender_id");
    };
    let n = conn
        .execute(
            "DELETE FROM allowlist WHERE channel=?1 AND sender_id=?2",
            params![channel, sender_id],
        )
        .unwrap_or(0);
    ok_response(id, json!({"channel": channel, "sender_id": sender_id, "revoked": n > 0}))
}

// ── channel.lockdown — 전 채널 즉시 정지·인바운드 전면 차단(§2.4-5) ───────────

fn lockdown(daemon: &Arc<Daemon>, conn: &mut Connection, id: &Value) -> Value {
    let rows: Vec<(Option<i64>, Option<i64>)> = conn
        .prepare("SELECT scoped_pid, scoped_pgid FROM channels WHERE enabled=1")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    let mut killed = 0;
    for (pid, pgid) in &rows {
        if let Some(pid) = pid {
            crate::governance::kill_group_or_pid(*pid as u32, pgid.unwrap_or(*pid) as i32);
            daemon.ledger.lock().unwrap().remove(&(*pid as u32));
            killed += 1;
        }
    }
    let _ = conn.execute(
        "UPDATE channels SET enabled=0, registered=0, scoped_pid=NULL, scoped_pgid=NULL, updated_ts=?1",
        params![now()],
    );
    meta_set(conn, "lockdown", "1");
    daemon.bus.publish(
        "channel.lockdown",
        "channel",
        None,
        json!({"channels_stopped": rows.len(), "bridges_killed": killed}),
    );
    ok_response(id, json!({"lockdown": true, "channels_stopped": rows.len(), "bridges_killed": killed}))
}

// ── channel.unlock — lockdown 해제(one-way door 제거·H2) ──────────────────────

/// `cys channel unlock` → lockdown 플래그 해제(meta lockdown="0") + channel.unlocked 이벤트.
/// lockdown은 전 채널을 enabled=0으로 내리고 인바운드를 전면 차단하는데, 지금까지 "1" 쓰기만 있고
/// "0" 복원 경로가 없어 1회 잠금이 영구 불능(DB 수동편집 외 복구 불가)이었다. 이 RPC가 그 유일한
/// 해제 경로다. LOW-2: 인가 경계는 인바운드 차단 논리가 아니라 소켓 0o600 same-UID 봉인이다
/// (pause/resume 등 여타 RPC와 동일 신뢰층위 — 브리지는 lockdown 시 kill되고 소켓은 어차피 동일
/// UID 전용). 긴급 unlock에 추가 인가(오너 토큰/feed)를 원하면 별도 결합 — 현재는 미결합. 해제는 desired-state를
/// 되돌리지 않는다(enabled는 그대로 0) — 채널 재개는 `cys channel start`로 명시한다(안전측).
fn unlock(daemon: &Arc<Daemon>, conn: &mut Connection, id: &Value) -> Value {
    let was = lockdown_active(conn);
    meta_set(conn, "lockdown", "0");
    daemon.bus.publish(
        "channel.unlocked",
        "channel",
        None,
        json!({"was_locked": was}),
    );
    ok_response(id, json!({"unlocked": true, "was_locked": was}))
}

// ── channel.status ───────────────────────────────────────────────────────────

fn status(conn: &Connection, id: &Value) -> Value {
    let mut channels = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT channel, enabled, registered, scoped_pid, bridge_ver, last_in_ts, last_out_ts FROM channels ORDER BY channel",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, Option<f64>>(6)?,
            ))
        }) {
            for (channel, enabled, registered, pid, bridge_ver, last_in, last_out) in rows.flatten() {
                let alive = pid.map(|p| pid_alive(p as u32)).unwrap_or(false);
                // outcome 분포
                let mut outcomes = serde_json::Map::new();
                if let Ok(mut s2) = conn.prepare(
                    "SELECT outcome, COUNT(*) FROM outbound WHERE channel=?1 GROUP BY outcome",
                ) {
                    if let Ok(rr) = s2.query_map([&channel], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                        for (oc, c) in rr.flatten() {
                            outcomes.insert(oc, json!(c));
                        }
                    }
                }
                // inbox 상태 카운트
                let mut inbox = serde_json::Map::new();
                if let Ok(mut s3) = conn.prepare(
                    "SELECT state, COUNT(*) FROM inbox WHERE channel=?1 GROUP BY state",
                ) {
                    if let Ok(rr) = s3.query_map([&channel], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                        for (st, c) in rr.flatten() {
                            inbox.insert(st, json!(c));
                        }
                    }
                }
                let allow_n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM allowlist WHERE channel=?1", [&channel], |r| r.get(0))
                    .unwrap_or(0);
                // M12: 연속 재스폰 실패 카운터 + down 관측(임계 이상이면 채널 수준 down).
                let respawn_fails: i64 = meta_get(conn, &format!("respawn_fails:{channel}"))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let down = respawn_fails >= RESPAWN_FAIL_ALERT_THRESHOLD as i64;
                channels.push(json!({
                    "channel": channel, "enabled": enabled == 1, "registered": registered == 1,
                    "alive": alive, "pid": pid, "bridge_ver": bridge_ver,
                    "last_in_ts": last_in, "last_out_ts": last_out,
                    "outcomes": Value::Object(outcomes), "inbox": Value::Object(inbox),
                    "allowlist": allow_n, "respawn_fails": respawn_fails, "down": down,
                }));
            }
        }
    }
    let ra_until = remote_approve_until(conn);
    ok_response(
        id,
        json!({"channels": channels, "lockdown": lockdown_active(conn),
               "remote_approve": {"active": ra_until > now(), "until": ra_until}}),
    )
}

// ── 부팅 재조정(reconcile) — §2.1-2 ─────────────────────────────────────────

/// cysd 기동 시 enabled=1 채널마다: ①기록된 pgid 생존 시 고아 선-kill ②새 토큰으로 재스폰
/// ③channel.reconciled 발행. lockdown 중이면 재스폰 보류(안전측 — lockdown은 재부팅에도 유지).
pub fn reconcile(daemon: &Arc<Daemon>) {
    // LOW-3 H3 심층방어 — 채널 sweep/reconcile은 메인 cysd 전용(부서 데몬 no-op).
    if cys::is_dept_socket(&daemon.socket_path) {
        return;
    }
    let mut guard = daemon.channels.lock().unwrap();
    let Some(conn) = guard.as_mut() else {
        return;
    };
    if lockdown_active(conn) {
        eprintln!("[cysd] channels: lockdown active — reconcile skips respawn");
        return;
    }
    let rows: Vec<(String, Option<String>, Option<i64>, Option<i64>)> = conn
        .prepare("SELECT channel, bridge_cmd, scoped_pid, scoped_pgid FROM channels WHERE enabled=1")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    for (channel, bridge_cmd, old_pid, old_pgid) in rows {
        // ① 고아 선-kill(구 토큰 브리지의 이중 연결·블랙홀 차단).
        if let Some(pid) = old_pid {
            if pid_alive(pid as u32) {
                // MED-4: killpg 직전 정체 재확인. 브리지는 setsid로 스폰돼 pgid==pid(세션 리더)이므로,
                // 지금 관측한 pgid가 저장된 old_pgid와 일치(둘 다 Some & 상등)할 때만 kill한다.
                // 재사용된 pid는 우리 세션 리더 pgid를 거의 공유하지 않음 → pgid 일치가 정체 프록시.
                // 불일치=이미 죽은 브리지(스킵 안전·되살아난 무관 그룹 오살상 방지). old_pgid None(검증 불가)도 보수적 스킵.
                let now_pgid = pgid_of(pid as u32);
                match (old_pgid, now_pgid) {
                    (Some(stored), Some(cur)) if stored as i32 == cur => {
                        crate::governance::kill_group_or_pid(pid as u32, cur);
                    }
                    _ => {
                        eprintln!("[cysd] channels: '{channel}' pid {pid} pgid mismatch (stored {old_pgid:?}, now {now_pgid:?}) — stale pid reuse, skip kill");
                    }
                }
            }
            daemon.ledger.lock().unwrap().remove(&(pid as u32)); // stale 엔트리 정리(kill 여부 무관).
        }
        // ② 새 토큰으로 재스폰(bridge_cmd 있을 때만).
        let Some(cmd) = bridge_cmd else {
            eprintln!("[cysd] channels: '{channel}' enabled but no bridge_cmd — cannot respawn");
            continue;
        };
        let token = match random_token_hex() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[cysd] channels: '{channel}' token gen failed, skip reconcile: {e}");
                continue; // L2: 예측가능 폴백 금지 — 다음 sweep/재시도에서 다시 시도.
            }
        };
        let token_hash = hex_sha256(token.as_bytes());
        match spawn_bridge(daemon, &channel, &cmd, &token) {
            Ok((pid, pgid)) => {
                let _ = conn.execute(
                    "UPDATE channels SET scoped_pid=?2, scoped_pgid=?3, token_hash=?4, registered=0, last_spawn_ts=?5, updated_ts=?5 WHERE channel=?1",
                    params![channel, pid as i64, pgid as i64, token_hash, now()],
                );
                daemon.bus.publish(
                    "channel.reconciled",
                    "channel",
                    None,
                    json!({"channel": channel, "pid": pid, "reason": "boot reconcile"}),
                );
            }
            Err(e) => eprintln!("[cysd] channels: reconcile respawn '{channel}' failed: {e}"),
        }
    }
}

// ── 주기 sweep — 재배달·타임아웃·브리지 사망 재스폰 ──────────────────────────

/// outbound 타임아웃 전이(순수 SQL 캡슐) — pending이 timeout 창을 넘으면 unknown(단조).
fn sweep_outbound_timeouts(conn: &Connection, now: f64, timeout: f64) -> usize {
    conn.execute(
        "UPDATE outbound SET outcome='unknown', updated_ts=?1 WHERE outcome='pending' AND (?1 - created_ts) >= ?2",
        params![now, timeout],
    )
    .unwrap_or(0)
}

/// loopwin 오래된 행 프룬(윈도우 밖).
fn prune_loopwin(conn: &Connection, now: f64) {
    let _ = conn.execute(
        "DELETE FROM loopwin WHERE ts < ?1",
        params![now - LOOP_WINDOW_SECS * 2.0],
    );
}

/// M5: 종결 원장 행 보존기간 프룬(무한성장·disk DoS 차단). pending·최근 행·**미소각 approval_prompt
/// (M6 재조정 대상 = 살아있는 버튼)**는 보존한다. 삭제한 총 행수 반환(관측용).
fn prune_retention(conn: &Connection, now: f64, retain_secs: f64) -> usize {
    let cutoff = now - retain_secs;
    let mut n = 0;
    // terminal outbound(비-pending·updated_ts 기준) — 단, 미소각 approval_prompt는 살아있는 버튼이라 보존.
    n += conn
        .execute(
            "DELETE FROM outbound WHERE outcome!='pending' AND updated_ts < ?1
             AND NOT (kind='approval_prompt' AND approval_nonce_used=0)",
            params![cutoff],
        )
        .unwrap_or(0);
    // acked inbox(본문은 ack 때 이미 소거) — new/injected는 배달 대기라 보존.
    n += conn
        .execute(
            "DELETE FROM inbox WHERE state='acked' AND acked_ts IS NOT NULL AND acked_ts < ?1",
            params![cutoff],
        )
        .unwrap_or(0);
    // denied·loop_suppressed inbound(거부·억제 판정) — accepted는 감사 추적으로 보존.
    n += conn
        .execute(
            "DELETE FROM inbound WHERE verdict IN ('denied','loop_suppressed') AND ts < ?1",
            params![cutoff],
        )
        .unwrap_or(0);
    // late_receipt 화해 레코드.
    n += conn
        .execute("DELETE FROM late_receipt WHERE ts < ?1", params![cutoff])
        .unwrap_or(0);
    n
}

/// M12: 재스폰 실패 카운터 증가 — 임계 도달·미경보면 Some(fails)(=health.alert 발행 신호), 아니면
/// None. 경보는 임계 도달 시 **1회만**(respawn_alerted 플래그) — 무한 재시도 중 경보 폭주 차단.
fn bump_respawn_failure(conn: &Connection, channel: &str, threshold: u64) -> Option<u64> {
    let fk = format!("respawn_fails:{channel}");
    let fails = meta_get(conn, &fk).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) + 1;
    meta_set(conn, &fk, &fails.to_string());
    let ak = format!("respawn_alerted:{channel}");
    let already = meta_get(conn, &ak).as_deref() == Some("1");
    if fails >= threshold && !already {
        meta_set(conn, &ak, "1");
        Some(fails)
    } else {
        None
    }
}

/// M12: 재스폰 성공 시 실패 카운터·경보 플래그 리셋(다음 사망 시 다시 임계까지 카운트).
fn reset_respawn_failure(conn: &Connection, channel: &str) {
    meta_set(conn, &format!("respawn_fails:{channel}"), "0");
    meta_set(conn, &format!("respawn_alerted:{channel}"), "0");
}

/// enabled=1이지만 브리지 pid가 죽은 채널을 백오프 후 재스폰(§2.1-5).
fn respawn_dead_bridges(daemon: &Arc<Daemon>, conn: &mut Connection) {
    if lockdown_active(conn) {
        return;
    }
    let now = now();
    let rows: Vec<(String, Option<String>, Option<i64>, Option<f64>)> = conn
        .prepare("SELECT channel, bridge_cmd, scoped_pid, last_spawn_ts FROM channels WHERE enabled=1")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    for (channel, bridge_cmd, pid, last_spawn) in rows {
        let dead = pid.map(|p| !pid_alive(p as u32)).unwrap_or(true);
        if !dead {
            continue;
        }
        // 백오프: 직전 스폰 후 RESPAWN_BACKOFF_SECS 미만이면 대기(크래시 루프 폭주 차단).
        if last_spawn.map(|t| now - t < RESPAWN_BACKOFF_SECS).unwrap_or(false) {
            continue;
        }
        let Some(cmd) = bridge_cmd else {
            continue;
        };
        let token = match random_token_hex() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[cysd] channels: '{channel}' token gen failed, skip respawn: {e}");
                continue; // L2: 예측가능 폴백 금지.
            }
        };
        let token_hash = hex_sha256(token.as_bytes());
        match spawn_bridge(daemon, &channel, &cmd, &token) {
            Ok((npid, npgid)) => {
                reset_respawn_failure(conn, &channel); // M12: 성공 → 실패 카운터·경보 리셋.
                let _ = conn.execute(
                    "UPDATE channels SET scoped_pid=?2, scoped_pgid=?3, token_hash=?4, registered=0, last_spawn_ts=?5, updated_ts=?5 WHERE channel=?1",
                    params![channel, npid as i64, npgid as i64, token_hash, now],
                );
                daemon.bus.publish(
                    "channel.reconciled",
                    "channel",
                    None,
                    json!({"channel": channel, "pid": npid, "reason": "respawn after bridge death"}),
                );
            }
            Err(e) => {
                // M12: 연속 실패 카운터 → 임계 초과 시 health.alert 1회(무한 재시도는 유지·관측 가능).
                // last_spawn_ts를 갱신해 백오프 창을 유지(폭주 차단) — 실패해도 즉시 재시도 안 함.
                let _ = conn.execute(
                    "UPDATE channels SET last_spawn_ts=?2, updated_ts=?2 WHERE channel=?1",
                    params![channel, now],
                );
                if let Some(fails) = bump_respawn_failure(conn, &channel, RESPAWN_FAIL_ALERT_THRESHOLD) {
                    daemon.bus.publish(
                        "health.alert",
                        "health",
                        None,
                        json!({"rule": "channel_bridge_down", "channel": channel,
                               "fails": fails, "detail": e}),
                    );
                }
            }
        }
    }
}

/// sweep 1틱(동기) — 테스트가 직접 호출 가능. 재배달·타임아웃·프룬·재스폰.
fn sweep_once(daemon: &Arc<Daemon>) {
    // LOW-3 H3 심층방어 — 채널 sweep/reconcile은 메인 cysd 전용(부서 데몬 no-op).
    if cys::is_dept_socket(&daemon.socket_path) {
        return;
    }
    let mut guard = daemon.channels.lock().unwrap();
    let Some(conn) = guard.as_mut() else {
        return;
    };
    let now = now();
    // pause(§2.6 O5) 중엔 outbound 타임아웃 시계도 동결한다 — 배달을 의도적으로 얼린 동안
    // pending을 unknown(terminal)으로 넘기면 resume_flush(=pending만 재발행)가 그 항목을 잃는다.
    // deliver_new_inbox/redeliver_unacked는 deliverable_master의 pause 게이트로 자연 no-op이고,
    // respawn은 pause와 무관하게 계속(inbound 판정에 브리지가 살아있어야 하므로).
    if !daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
        sweep_outbound_timeouts(conn, now, outbound_unknown_secs());
    }
    prune_loopwin(conn, now);
    prune_retention(conn, now, channel_retain_secs()); // M5: 종결 원장 보존기간 프룬.
    // new 배달 → un-acked 재배달(순서: 신규 우선, 그 다음 재배달).
    deliver_new_inbox(daemon, conn);
    redeliver_unacked(daemon, conn);
    respawn_dead_bridges(daemon, conn);
}

/// 주기 sweep 태스크 등록(governance watchdog 패턴). main.rs 초기화에서 spawn.
pub fn spawn_channel_sweep(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS));
        loop {
            tick.tick().await;
            sweep_once(&daemon);
        }
    });
}

/// pause 해제(§2.6 O5) 시 동결분 방출 — `system.resume` 핸들러에서 `paused=false` 확정 **후** 호출.
/// ① pause 중 발행이 동결된 pending 아웃바운드 이벤트를 채널별로 재발행(브리지가 배달 지시 재수신·
///    outbound_id로 dedupe — at-least-once). ② pause 중 적재만 되고 주입 보류된 inbox를 드레인한다.
/// 호출 전 이미 paused=false이므로 deliverable_master의 pause 게이트는 통과한다.
pub fn resume_flush(daemon: &Arc<Daemon>) {
    let mut guard = daemon.channels.lock().unwrap();
    let Some(conn) = guard.as_mut() else {
        return;
    };
    // ① 동결된 pending 아웃바운드 재발행(enabled 채널 한정 — stop된 채널의 잔여는 방출 안 함).
    let channels: Vec<String> = conn
        .prepare("SELECT channel FROM channels WHERE enabled=1")
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default();
    for ch in channels {
        for mut ob in pending_outbound(conn, &ch) {
            if let Some(obj) = ob.as_object_mut() {
                obj.insert("channel".into(), json!(ch));
            }
            daemon
                .bus
                .publish(&format!("channel.outbound.{ch}"), "channel", None, ob);
        }
    }
    // ② pause 중 보류된 inbox 드레인(신규 우선→un-acked 재배달, sweep_once와 동일 순서).
    deliver_new_inbox(daemon, conn);
    redeliver_unacked(daemon, conn);
}

// ── 테스트 ───────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Daemon;

    fn tmp_daemon(tag: &str) -> Arc<Daemon> {
        // ★R5-B: 이 데몬을 거치는 주입은 `inject_master` 에서 배달 원장에 append 된다
        // (Origin::Channel). 격리를 안 걸면 `pack_state_dir()` 이 실 HOME 으로 해소돼
        // `~/.cys/state/delivery-<임시소켓>.jsonl` 이 실제로 생긴다 — 하네스에서 일괄로 접는다.
        crate::delivery::tests::isolate_state_dir_for_thread(tag);
        let dir = std::env::temp_dir().join(format!("cys_chan_test_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Daemon::new(dir.join("cysd.sock"))
    }

    fn call(daemon: &Arc<Daemon>, sub: &str, params: Value, caller_pid: Option<u32>) -> Value {
        match handle(daemon, sub, &params, &json!(1), caller_pid) {
            Reply::Single(v) => v,
            _ => panic!("expected Single reply"),
        }
    }

    /// 채널을 registered 상태로 만드는 헬퍼(실스폰 없이 DB 직삽입). scoped_pgid=이 프로세스 그룹
    /// → inbound/register의 pid 핀이 own pid로 통과. 토큰은 known.
    fn seed_registered(daemon: &Arc<Daemon>, channel: &str, token: &str) {
        let own_pgid = own_pgid();
        let mut g = daemon.channels.lock().unwrap();
        let conn = g.as_mut().unwrap();
        let th = hex_sha256(token.as_bytes());
        conn.execute(
            "INSERT INTO channels(channel, enabled, bridge_cmd, scoped_pid, scoped_pgid, token_hash, registered, updated_ts)
             VALUES(?1,1,'true',?2,?3,?4,1,?5)",
            params![channel, std::process::id() as i64, own_pgid, th, now()],
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn own_pgid() -> i64 {
        unsafe { libc::getpgid(0) as i64 }
    }
    #[cfg(windows)]
    fn own_pgid() -> i64 {
        0
    }
    fn own_pid() -> Option<u32> {
        Some(std::process::id())
    }

    fn inbound_params(channel: &str, sender: &str, key: &str, text: &str, kind: &str) -> Value {
        json!({"channel": channel, "sender_id": sender, "sender_kind": kind,
               "idempotency_key": key, "text": text})
    }

    #[test]
    fn open_creates_schema_and_version() {
        let d = tmp_daemon("schema");
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().expect("channels db open");
        let v: String = conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "3"); // OPP-21 v3(원격승인 상한·쿨다운 테이블 추가).
    }

    // C1: 채널 인바운드 살균 — paste-escape·CR·ESC·C0/DEL 제거, 한글·이모지·개행·탭 보존.
    #[test]
    fn sanitize_strips_paste_escape_preserves_text() {
        // bracketed-paste 종료 시퀀스 strip(경계 붕괴 차단).
        assert_eq!(sanitize_inbound_text("\x1b[201~rm -rf"), "[201~rm -rf");
        // ESC·CR·기타 C0·DEL 제거.
        assert_eq!(sanitize_inbound_text("a\rb\x00c\x1bd\x7fe"), "abcde");
        assert_eq!(sanitize_inbound_text("x\x1b[200~y"), "x[200~y");
        // 정상 한글/이모지/개행/탭 무손상.
        assert_eq!(sanitize_inbound_text("안녕하세요 🚀\n다음\t줄"), "안녕하세요 🚀\n다음\t줄");
        // 봉투에 ESC/CR 유입 0(수용기준).
        let env = envelope("slack", "U1", 0.0, 7, false, "hi\x1b[201~\r\nX");
        assert!(!env.contains('\x1b'), "봉투에 ESC 유입: {env:?}");
        assert!(!env.contains('\r'), "봉투에 CR 유입: {env:?}");
        assert!(env.contains("#7"), "봉투 구조 보존: {env:?}");
    }

    // MED-1: 화이트리스트화 — 8비트 C1(CSI `\u{9b}`·NEL `\u{85}`)·LS/PS 제거, 정상문자 보존.
    #[test]
    fn sanitize_strips_c1_and_line_separators() {
        // 8비트 CSI(`\u{9b}201~`) = 8비트 bracketed-paste 종료 → 제거.
        assert_eq!(sanitize_inbound_text("\u{9b}201~rm -rf"), "201~rm -rf");
        // NEL(`\u{85}`) 및 C1 경계(`\u{80}`·`\u{9f}`) 제거.
        assert_eq!(sanitize_inbound_text("a\u{85}b\u{80}c\u{9f}d"), "abcd");
        // 줄/문단 구분자(U+2028 LS·U+2029 PS) 제거.
        assert_eq!(sanitize_inbound_text("x\u{2028}y\u{2029}z"), "xyz");
        // 한글·이모지·탭·개행은 보존.
        assert_eq!(sanitize_inbound_text("한글\t이모지🚀\n끝"), "한글\t이모지🚀\n끝");
    }

    // LOW-1: envelope의 sender·channel도 살균 — ESC/C1을 담은 sender/channel이 봉투에서 제거된다.
    #[test]
    fn envelope_sanitizes_sender_and_channel() {
        let env = envelope("sl\x1back", "U\u{9b}201~1", 0.0, 3, false, "hi");
        assert!(!env.contains('\x1b'), "봉투 channel에 ESC 유입: {env:?}");
        assert!(!env.contains('\u{9b}'), "봉투 sender에 8비트 CSI 유입: {env:?}");
        // 살균 후 정상 잔여 문자는 보존.
        assert!(env.contains("slack"), "channel 정상문자 보존: {env:?}");
        assert!(env.contains("#3"), "봉투 구조 보존: {env:?}");
    }

    // H2: lockdown → unlock → inbound 복원 + reconcile 재개(플래그 OFF).
    #[test]
    fn lockdown_then_unlock_restores_inbound() {
        let d = tmp_daemon("unlock");
        // scoped_pid=NULL로 seed — lockdown의 브리지 kill이 실프로세스(테스트 자신)를 죽이지 않게.
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute(
                "INSERT INTO channels(channel, enabled, bridge_cmd, scoped_pid, scoped_pgid, token_hash, registered, updated_ts)
                 VALUES('slack',1,'true',NULL,NULL,?1,1,?2)",
                params![hex_sha256(b"t"), now()],
            )
            .unwrap();
        }
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // lockdown: 전역 플래그 ON + 채널 enabled=0(scoped_pid NULL이라 kill 없음).
        let lk = call(&d, "lockdown", json!({}), None);
        assert_eq!(lk["result"]["lockdown"], json!(true), "{lk}");
        {
            let g = d.channels.lock().unwrap();
            assert!(lockdown_active(g.as_ref().unwrap()), "lockdown 후 플래그 ON — reconcile 보류");
        }
        // unlock: 플래그 OFF(reconcile 재개 조건) + was_locked 보고.
        let un = call(&d, "unlock", json!({}), None);
        assert_eq!(un["result"]["unlocked"], json!(true), "{un}");
        assert_eq!(un["result"]["was_locked"], json!(true), "{un}");
        {
            let g = d.channels.lock().unwrap();
            assert!(!lockdown_active(g.as_ref().unwrap()), "unlock 후 플래그 OFF — reconcile 재개");
        }
        // 채널 재개(start가 desired-state 복원) 시뮬레이션 — lockdown이 내린 enabled/registered 복원.
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute(
                "UPDATE channels SET enabled=1, registered=1, scoped_pid=?1, scoped_pgid=?2 WHERE channel='slack'",
                params![std::process::id() as i64, own_pgid()],
            )
            .unwrap();
        }
        // inbound가 다시 inbox에 적재됨(정상 복원 — 원격 잠금이 영구 불능이 아님).
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:after", "hello", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("queued"), "unlock 후 inbound 복원: {r}");
    }

    // H3: 부서 데몬은 channel.start·register를 구조적으로 거부(메인 단독 소유). 메인은 오판 없음.
    fn tmp_daemon_dept(tag: &str) -> Arc<Daemon> {
        let dir = std::env::temp_dir().join(format!("cys-dept-chtest-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Daemon::new(dir.join("cys.sock"))
    }

    #[test]
    fn dept_daemon_forbids_channel_spawn_main_allows() {
        let d = tmp_daemon_dept("deptreject");
        let s = call(&d, "start", json!({"channel": "slack", "cmd": "true"}), None);
        assert_eq!(s["ok"], json!(false), "{s}");
        assert_eq!(s["error"]["code"], json!("dept_channel_forbidden"), "부서 start 거부: {s}");
        let r = call(&d, "register", json!({"channel": "slack", "token": "t"}), own_pid());
        assert_eq!(r["error"]["code"], json!("dept_channel_forbidden"), "부서 register 거부: {r}");
        // 메인 데몬(비-부서)은 dept 거부가 아니라 정상 경로 진입 — register는 미기동이라 not_started
        // (dept_channel_forbidden이 아님을 확인 = 메인 오판 금지). start 스폰은 회피(register로 증명).
        let m = tmp_daemon("main_notdept");
        let mr = call(&m, "register", json!({"channel": "slack", "token": "t"}), own_pid());
        assert_eq!(mr["error"]["code"], json!("not_started"), "메인은 dept 거부 아닌 정상 경로: {mr}");
    }

    #[test]
    fn register_rejects_token_mismatch() {
        let d = tmp_daemon("regtok");
        // enabled 채널, token_hash=sha256("goodtoken"), scoped_pgid=own group.
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute(
                "INSERT INTO channels(channel, enabled, scoped_pid, scoped_pgid, token_hash, registered, updated_ts)
                 VALUES('slack',1,?1,?2,?3,0,?4)",
                params![std::process::id() as i64, own_pgid(), hex_sha256(b"goodtoken"), now()],
            )
            .unwrap();
        }
        let bad = call(&d, "register", json!({"channel": "slack", "token": "wrongtoken"}), own_pid());
        assert_eq!(bad["ok"], json!(false), "위장 토큰 등록이 거부돼야 한다: {bad}");
        assert_eq!(bad["error"]["code"], json!("auth_denied"));
    }

    #[cfg(unix)]
    #[test]
    fn register_rejects_non_spawn_pid() {
        let d = tmp_daemon("regpid");
        // scoped_pgid를 존재하지 않는 그룹(999999)로 → own pid의 pgid와 불일치 → 거부.
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute(
                "INSERT INTO channels(channel, enabled, scoped_pid, scoped_pgid, token_hash, registered, updated_ts)
                 VALUES('slack',1,1234,999999,?1,0,?2)",
                params![hex_sha256(b"tok"), now()],
            )
            .unwrap();
        }
        let r = call(&d, "register", json!({"channel": "slack", "token": "tok"}), own_pid());
        assert_eq!(r["ok"], json!(false), "비스폰 pid 등록이 거부돼야 한다: {r}");
        assert_eq!(r["error"]["code"], json!("auth_denied"));
    }

    #[test]
    fn register_returns_pending_outbound() {
        let d = tmp_daemon("regpend");
        seed_registered(&d, "slack", "t");
        // allowlist + pending outbound 하나 만들어둔다.
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let ob = call(&d, "outbound", json!({"channel": "slack", "target": "U1", "body": "hi", "idempotency_key": "o1"}), None);
        assert_eq!(ob["result"]["outcome"], json!("pending"));
        // 재등록(토큰 일치·own pid) → 응답에 pending 동봉.
        let reg = call(&d, "register", json!({"channel": "slack", "token": "t", "bridge_ver": "v0"}), own_pid());
        assert_eq!(reg["ok"], json!(true), "{reg}");
        let pend = reg["result"]["pending"].as_array().unwrap();
        assert_eq!(pend.len(), 1, "pending outbound 동봉돼야 한다: {reg}");
        assert_eq!(pend[0]["idempotency_key"], json!("o1"));
    }

    // M12: 재스폰 실패 카운터 → 임계서 health.alert 신호 1회, 성공 리셋 후 다시 임계서 재신호.
    #[test]
    fn respawn_failure_counter_alerts_once_then_resets() {
        let d = tmp_daemon("m12fail");
        let mut g = d.channels.lock().unwrap();
        let conn = g.as_mut().unwrap();
        // 임계=3: 1·2회 None, 3회 Some(3)=경보 발행 신호, 이후 None(1회만).
        assert_eq!(bump_respawn_failure(conn, "slack", 3), None);
        assert_eq!(bump_respawn_failure(conn, "slack", 3), None);
        assert_eq!(bump_respawn_failure(conn, "slack", 3), Some(3), "임계 도달 시 경보 신호");
        assert_eq!(bump_respawn_failure(conn, "slack", 3), None, "임계 후 재경보 억제(폭주 차단)");
        // 성공 리셋 → 카운터·경보 플래그 해제.
        reset_respawn_failure(conn, "slack");
        assert_eq!(
            meta_get(conn, "respawn_fails:slack").as_deref(),
            Some("0"),
            "리셋 후 카운터 0"
        );
        assert_eq!(bump_respawn_failure(conn, "slack", 3), None);
        assert_eq!(bump_respawn_failure(conn, "slack", 3), None);
        assert_eq!(bump_respawn_failure(conn, "slack", 3), Some(3), "리셋 후 다시 임계서 경보");
    }

    // M5: 종결 원장 보존기간 프룬 — 오래된 종결행 삭제·pending/최근/미소각 approval/accepted 보존.
    #[test]
    fn prune_retention_removes_old_terminal_keeps_live() {
        let d = tmp_daemon("m5prune");
        seed_registered(&d, "slack", "t");
        let old = now() - 100.0 * 86400.0; // 100일 전(보존기간 7일 초과)
        let fresh = now();
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            // outbound: old sent(프룬)·old pending(보존)·fresh sent(보존)·old 미소각 approval(보존).
            conn.execute("INSERT INTO outbound(channel,target,kind,body,idempotency_key,outcome,created_ts,updated_ts) VALUES('slack','U1','message','x','o_old','sent',?1,?1)", params![old]).unwrap();
            conn.execute("INSERT INTO outbound(channel,target,kind,body,idempotency_key,outcome,created_ts,updated_ts) VALUES('slack','U1','message','x','o_pending','pending',?1,?1)", params![old]).unwrap();
            conn.execute("INSERT INTO outbound(channel,target,kind,body,idempotency_key,outcome,created_ts,updated_ts) VALUES('slack','U1','message','x','o_fresh','sent',?1,?1)", params![fresh]).unwrap();
            conn.execute("INSERT INTO outbound(channel,target,kind,body,idempotency_key,outcome,approval_nonce_used,created_ts,updated_ts) VALUES('slack','U1','approval_prompt','x','a_live','sent',0,?1,?1)", params![old]).unwrap();
            // inbox: old acked(프룬)·old new(보존).
            conn.execute("INSERT INTO inbox(channel,sender_id,text,state,created_ts,acked_ts) VALUES('slack','U1','','acked',?1,?1)", params![old]).unwrap();
            conn.execute("INSERT INTO inbox(channel,sender_id,text,state,created_ts) VALUES('slack','U1','hi','new',?1)", params![old]).unwrap();
            // inbound: old denied(프룬)·old accepted(보존).
            conn.execute("INSERT INTO inbound(channel,idempotency_key,body_hash,ts,verdict) VALUES('slack','k_old','h',?1,'denied')", params![old]).unwrap();
            conn.execute("INSERT INTO inbound(channel,idempotency_key,body_hash,ts,verdict) VALUES('slack','k_acc','h',?1,'accepted')", params![old]).unwrap();
            // late_receipt: old(프룬).
            conn.execute("INSERT INTO late_receipt(outbound_id,outcome,ts) VALUES(1,'sent',?1)", params![old]).unwrap();

            let removed = prune_retention(conn, now(), channel_retain_secs());
            assert!(removed >= 4, "오래된 종결행 최소 4건 프룬: {removed}");

            let has_ob = |k: &str| -> bool {
                conn.query_row("SELECT 1 FROM outbound WHERE idempotency_key=?1", [k], |_| Ok(())).optional().unwrap().is_some()
            };
            assert!(!has_ob("o_old"), "오래된 sent 프룬");
            assert!(has_ob("o_pending"), "pending 보존");
            assert!(has_ob("o_fresh"), "최근 sent 보존");
            assert!(has_ob("a_live"), "미소각 approval(살아있는 버튼) 보존");
            let inbox_new: i64 = conn.query_row("SELECT COUNT(*) FROM inbox WHERE state='new'", [], |r| r.get(0)).unwrap();
            let inbox_acked: i64 = conn.query_row("SELECT COUNT(*) FROM inbox WHERE state='acked'", [], |r| r.get(0)).unwrap();
            assert_eq!(inbox_new, 1, "new inbox 보존");
            assert_eq!(inbox_acked, 0, "오래된 acked inbox 프룬");
            let denied: i64 = conn.query_row("SELECT COUNT(*) FROM inbound WHERE verdict='denied'", [], |r| r.get(0)).unwrap();
            let accepted: i64 = conn.query_row("SELECT COUNT(*) FROM inbound WHERE verdict='accepted'", [], |r| r.get(0)).unwrap();
            assert_eq!(denied, 0, "오래된 denied 프룬");
            assert_eq!(accepted, 1, "accepted 보존(감사 추적)");
            let lr: i64 = conn.query_row("SELECT COUNT(*) FROM late_receipt", [], |r| r.get(0)).unwrap();
            assert_eq!(lr, 0, "오래된 late_receipt 프룬");
        }
    }

    // M6: 재스폰 후 register가 sent-but-feed-pending approval_prompt를 재조정 목록에 포함(버튼 복원).
    #[test]
    fn register_reprompts_sent_approval_with_pending_feed() {
        let d = tmp_daemon("m6reprompt");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // pending feed 하나.
        d.feed_items.lock().unwrap().push(crate::state::FeedItem {
            request_id: "F1".into(),
            kind: "permission".into(),
            title: "t".into(),
            body: "b".into(),
            surface_id: None,
            status: "pending".into(),
            decision: None,
            created_at: now(),
            resolved_at: None,
            tier: Some("c".into()),
            publisher_pid: None,
            publisher_pgid: None,
            publisher_surface: None,
            risk_class: None,
            auto_route: false,
        });
        // 이미 sent된 approval_prompt(feed F1·nonce 미소각).
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute(
                "INSERT INTO outbound(channel,target,kind,body,reply_to,idempotency_key,retry_of,outcome,
                                      approval_feed_id,approval_nonce,approval_owner,approval_nonce_used,created_ts,updated_ts)
                 VALUES('slack','U1','approval_prompt','[승인]',NULL,'approval:F1:U1',NULL,'sent','F1','nonceA','U1',0,?1,?1)",
                params![now()],
            )
            .unwrap();
        }
        // register → sent-but-pending approval 재조정 포함.
        let reg = call(&d, "register", json!({"channel": "slack", "token": "t", "bridge_ver": "v0"}), own_pid());
        assert_eq!(reg["ok"], json!(true), "{reg}");
        let pend = reg["result"]["pending"].as_array().unwrap();
        assert!(
            pend.iter().any(|p| p["idempotency_key"] == json!("approval:F1:U1")),
            "sent approval(feed pending) 재조정 포함: {reg}"
        );
        // feed 해소 → 재조정 제외.
        d.resolve_feed_item("F1", "allow");
        let reg2 = call(&d, "register", json!({"channel": "slack", "token": "t"}), own_pid());
        let pend2 = reg2["result"]["pending"].as_array().unwrap();
        assert!(
            !pend2.iter().any(|p| p["idempotency_key"] == json!("approval:F1:U1")),
            "해소된 feed의 approval은 재조정 제외: {reg2}"
        );
    }

    #[test]
    fn inbound_owner_only_fail_closed() {
        let d = tmp_daemon("owner");
        seed_registered(&d, "slack", "t");
        // allowlist 비어있음 → deny.
        let r = call(&d, "inbound", inbound_params("slack", "U_evil", "slack:1", "rm -rf", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("deny"), "allowlist 밖 sender는 deny: {r}");
        // 원장에 denied verdict 기록.
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let verdict: String = conn
            .query_row("SELECT verdict FROM inbound WHERE idempotency_key='slack:1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(verdict, "denied");
        // inbox엔 적재되지 않음.
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM inbox", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "deny된 메시지는 inbox에 적재되지 않는다");
    }

    #[test]
    fn inbound_owner_allowed_lands_in_inbox() {
        let d = tmp_daemon("inbox_land");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // master 없음 → queued.
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hello", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("queued"), "master 부재 시 queued: {r}");
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let (state, text): (String, String) = conn
            .query_row("SELECT state, text FROM inbox WHERE id=?1", [r["result"]["inbox_id"].as_i64().unwrap()], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(state, "new");
        assert_eq!(text, "hello");
    }

    #[test]
    fn inbound_idempotent_same_key_same_hash() {
        let d = tmp_daemon("idem_in");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let a = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hello", "user"), own_pid());
        assert_eq!(a["result"]["action"], json!("queued"));
        // 같은 key·같은 텍스트 재전송 → dup(신규 inbox 없음).
        let b = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hello", "user"), own_pid());
        assert_eq!(b["result"]["action"], json!("dup"), "{b}");
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM inbox", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "dup은 새 inbox를 만들지 않는다");
    }

    #[test]
    fn inbound_dedup_conflict_same_key_diff_hash() {
        let d = tmp_daemon("idem_conf");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hello", "user"), own_pid());
        // 같은 key·다른 텍스트 → dedup_conflict 에러.
        let c = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "DIFFERENT", "user"), own_pid());
        assert_eq!(c["ok"], json!(false), "{c}");
        assert_eq!(c["error"]["code"], json!("dedup_conflict"));
    }

    #[test]
    fn inbound_bot_loop_suppressed() {
        let d = tmp_daemon("botloop");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "bot1"}), None);
        // 21건(LOOP_LIMIT=20 초과)째부터 suppressed.
        let mut suppressed_seen = false;
        for i in 0..25 {
            let r = call(&d, "inbound", inbound_params("slack", "bot1", &format!("slack:{i}"), &format!("m{i}"), "bot"), own_pid());
            if r["result"]["action"] == json!("suppressed") {
                suppressed_seen = true;
            }
        }
        assert!(suppressed_seen, "봇 20건/60s 초과분은 suppressed 돼야 한다");
    }

    // M9: 비owner sender는 봇루프보다 앞선 owner-only에서 즉시 deny — loopwin 행 미생성.
    #[test]
    fn non_owner_denied_before_botloop_no_loopwin() {
        let d = tmp_daemon("m9order");
        seed_registered(&d, "slack", "t");
        // U_bot는 allowlist 밖(비owner) + sender_kind=bot.
        let r = call(&d, "inbound", inbound_params("slack", "U_bot", "slack:1", "spam", "bot"), own_pid());
        assert_eq!(r["result"]["action"], json!("deny"), "비owner bot는 봇루프 앞에서 즉시 deny: {r}");
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM loopwin", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "비owner deny 시 loopwin 미생성(자원 절약)");
        let v: String = conn
            .query_row("SELECT verdict FROM inbound WHERE idempotency_key='slack:1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "denied");
    }

    // L1: lockdown 중 outbound 게이트 — 행 생성·이벤트 차단.
    #[test]
    fn outbound_blocked_during_lockdown() {
        let d = tmp_daemon("l1lockout");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        {
            let mut g = d.channels.lock().unwrap();
            meta_set(g.as_mut().unwrap(), "lockdown", "1"); // 직접 set(브리지 kill 회피).
        }
        let r = call(&d, "outbound", json!({"channel": "slack", "target": "U1", "body": "x", "idempotency_key": "o1"}), None);
        assert_eq!(r["ok"], json!(false), "{r}");
        assert_eq!(r["error"]["code"], json!("lockdown_active"), "{r}");
        let g = d.channels.lock().unwrap();
        let n: i64 = g.as_ref().unwrap().query_row("SELECT COUNT(*) FROM outbound", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "lockdown 중 outbound 행 미생성");
    }

    #[test]
    fn outbound_monotonic_and_idempotent() {
        let d = tmp_daemon("ob_mono");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let a = call(&d, "outbound", json!({"channel": "slack", "target": "U1", "body": "x", "idempotency_key": "o1"}), None);
        let oid = a["result"]["outbound_id"].as_i64().unwrap();
        assert_eq!(a["result"]["outcome"], json!("pending"));
        // 같은 key 재호출 → 기존 outcome·id(신규 없음).
        let b = call(&d, "outbound", json!({"channel": "slack", "target": "U1", "body": "x", "idempotency_key": "o1"}), None);
        assert_eq!(b["result"]["outbound_id"].as_i64().unwrap(), oid);
        assert_eq!(b["result"]["idempotent"], json!(true));
        // receipt: pending→sent.
        let s = call(&d, "receipt", json!({"outbound_id": oid, "outcome": "sent", "platform_ref": "ts123"}), None);
        assert_eq!(s["result"]["outcome"], json!("sent"));
        // 늦은 receipt(다른 outcome) → 상태 불변 + late.
        let late = call(&d, "receipt", json!({"outbound_id": oid, "outcome": "failed"}), None);
        assert_eq!(late["result"]["late"], json!(true), "{late}");
        assert_eq!(late["result"]["outcome"], json!("sent"), "terminal 후 상태 불변");
        // late_receipt 레코드 존재.
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let ln: i64 = conn.query_row("SELECT COUNT(*) FROM late_receipt WHERE outbound_id=?1", [oid], |r| r.get(0)).unwrap();
        assert_eq!(ln, 1);
    }

    #[test]
    fn outbound_target_fail_closed() {
        let d = tmp_daemon("ob_fc");
        seed_registered(&d, "slack", "t");
        // allowlist 비어있음 → outbound 대상 거부.
        let r = call(&d, "outbound", json!({"channel": "slack", "target": "U_x", "body": "x", "idempotency_key": "o1"}), None);
        assert_eq!(r["ok"], json!(false), "{r}");
        assert_eq!(r["error"]["code"], json!("target_not_allowed"));
    }

    #[test]
    fn outbound_timeout_to_unknown() {
        let d = tmp_daemon("ob_to");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let a = call(&d, "outbound", json!({"channel": "slack", "target": "U1", "body": "x", "idempotency_key": "o1"}), None);
        let oid = a["result"]["outbound_id"].as_i64().unwrap();
        // 타임아웃 상수 주입(0초) → 즉시 unknown 전이.
        let mut g = d.channels.lock().unwrap();
        let conn = g.as_mut().unwrap();
        let n = sweep_outbound_timeouts(conn, now() + 1.0, 0.0);
        assert_eq!(n, 1);
        let oc: String = conn.query_row("SELECT outcome FROM outbound WHERE id=?1", [oid], |r| r.get(0)).unwrap();
        assert_eq!(oc, "unknown", "receipt 부재 시 unknown(단조·미확인)");
    }

    #[test]
    fn redeliver_due_pure() {
        assert!(redeliver_due("new", None, 100.0, 600.0));
        assert!(!redeliver_due("injected", Some(100.0), 200.0, 600.0));
        assert!(redeliver_due("injected", Some(100.0), 800.0, 600.0));
        assert!(!redeliver_due("acked", Some(100.0), 9999.0, 600.0));
    }

    #[test]
    fn envelope_shape() {
        let e = envelope("slack", "U0123", 0.0, 7, false, "hello");
        assert!(e.starts_with("[CH:slack|U0123|"), "{e}");
        assert!(e.ends_with("|#7] hello"), "{e}");
        let r = envelope("slack", "U0123", 0.0, 7, true, "hi");
        assert!(r.contains("(재배달)"), "{r}");
    }

    #[test]
    fn ack_marks_acked_and_clears_text() {
        let d = tmp_daemon("ack");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "secret", "user"), own_pid());
        let inbox_id = r["result"]["inbox_id"].as_i64().unwrap();
        let a = call(&d, "ack", json!({"inbox_id": inbox_id}), None);
        assert_eq!(a["result"]["acked"], json!(true), "{a}");
        // DB 검사 가드는 후속 call 전에 반드시 드롭한다(std Mutex 비재진입 — 자기 데드락 방지).
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let (state, text): (String, String) = conn
                .query_row("SELECT state, text FROM inbox WHERE id=?1", [inbox_id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap();
            assert_eq!(state, "acked");
            assert_eq!(text, "", "ack 시 본문 소거(프라이버시)");
        }
        // 이미 acked → not_found.
        let again = call(&d, "ack", json!({"inbox_id": inbox_id}), None);
        assert_eq!(again["ok"], json!(false));
    }

    #[cfg(unix)]
    #[test]
    fn lockdown_stops_and_blocks_inbound() {
        let d = tmp_daemon("lock");
        // 실제 더미 브리지 스폰(setsid로 자체 그룹 — 테스트 프로세스 그룹과 분리돼 kill이 안전).
        let st = call(&d, "start", json!({"channel": "slack", "cmd": "sleep 30"}), None);
        assert_eq!(st["ok"], json!(true), "{st}");
        let pid = st["result"]["pid"].as_u64().unwrap() as u32;
        assert!(pid_alive(pid), "스폰된 브리지가 살아있어야 한다");
        let l = call(&d, "lockdown", json!({}), None);
        assert_eq!(l["result"]["lockdown"], json!(true), "{l}");
        // 브리지 그룹 kill 확인(killpg 후 reaper 회수까지 짧은 유예 폴링).
        let mut gone = false;
        for _ in 0..100 {
            if !pid_alive(pid) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(gone, "lockdown이 브리지 그룹을 kill해야 한다");
        // 인바운드 전면 차단.
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:9", "hi", "user"), own_pid());
        assert_eq!(r["ok"], json!(false), "{r}");
        assert_eq!(r["error"]["code"], json!("locked"));
        // 채널 enabled=0 + lockdown 메타.
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let en: i64 = conn.query_row("SELECT enabled FROM channels WHERE channel='slack'", [], |r| r.get(0)).unwrap();
            assert_eq!(en, 0);
            assert!(lockdown_active(conn));
        }
    }

    #[test]
    fn revoke_removes_from_allowlist() {
        let d = tmp_daemon("revoke");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // 이제 revoke → 이후 인바운드 deny.
        let rv = call(&d, "revoke", json!({"channel": "slack", "sender_id": "U1"}), None);
        assert_eq!(rv["result"]["revoked"], json!(true));
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hi", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("deny"), "revoke 후 deny: {r}");
    }

    #[test]
    fn status_reports_channel_shape() {
        let d = tmp_daemon("status");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let s = call(&d, "status", json!({}), None);
        let chans = s["result"]["channels"].as_array().unwrap();
        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0]["channel"], json!("slack"));
        assert_eq!(chans[0]["registered"], json!(true));
        assert_eq!(chans[0]["allowlist"], json!(1));
        assert_eq!(s["result"]["lockdown"], json!(false));
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_cleans_dead_pgid_and_respawns() {
        let d = tmp_daemon("reconcile");
        // enabled=1, 죽은 pid(존재하지 않는 12345678), bridge_cmd=더미 sleep.
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute(
                "INSERT INTO channels(channel, enabled, bridge_cmd, scoped_pid, scoped_pgid, token_hash, registered, updated_ts)
                 VALUES('slack',1,'sleep 30',12345678,12345678,?1,1,?2)",
                params![hex_sha256(b"old"), now()],
            )
            .unwrap();
        }
        reconcile(&d);
        // 재스폰 → scoped_pid가 새 살아있는 pid로 교체·registered=0.
        let (pid, reg): (i64, i64) = {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            conn.query_row("SELECT scoped_pid, registered FROM channels WHERE channel='slack'", [], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
        };
        assert_ne!(pid, 12345678, "죽은 pid는 새 스폰으로 교체돼야 한다");
        assert!(pid_alive(pid as u32), "재스폰된 브리지가 살아있어야 한다");
        assert_eq!(reg, 0, "재스폰 후 registered=0(브리지 재등록 대기)");
        // 정리: 스폰한 sleep 그룹 회수.
        crate::governance::kill_group_or_pid(pid as u32, pid as i32);
    }

    #[cfg(unix)]
    #[test]
    fn inbox_delivered_then_redelivered_when_unacked() {
        let d = tmp_daemon("redeliver");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // 비-quiescing master surface(agent_status None → quiescing 아님).
        let surface = d
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("surface");
        d.roles.lock().unwrap().insert("master".into(), surface.id);
        // 인바운드 → 즉시 배달(delivered·state=injected).
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hi", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("delivered"), "master 가용 시 즉시 배달: {r}");
        let inbox_id = r["result"]["inbox_id"].as_i64().unwrap();
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let st: String = conn
                .query_row("SELECT state FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
                .unwrap();
            assert_eq!(st, "injected");
            // injected_ts를 TTL 초과 과거로 → 재배달 sweep 대상.
            conn.execute(
                "UPDATE inbox SET injected_ts=?2 WHERE id=?1",
                params![inbox_id, now() - INBOX_REDELIVER_TTL_SECS - 10.0],
            )
            .unwrap();
            let n = redeliver_unacked(&d, conn);
            assert_eq!(n, 1, "TTL 초과 un-acked 항목이 재배달돼야 한다");
            let redel: i64 = conn
                .query_row("SELECT redelivered FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
                .unwrap();
            assert_eq!(redel, 1, "재배달 표기(redelivered=1)");
        }
        // 정리: master surface의 sleep 30 회수.
        let _ = crate::governance::close_surface(&d, surface.id, crate::governance::CloseCause::Reap);
    }

    #[cfg(unix)]
    #[test]
    fn quiescing_master_holds_injection() {
        let d = tmp_daemon("quiesce");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // master surface 생성 + quiescing 자기보고.
        let surface = d
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("surface");
        let sid = surface.id;
        d.roles.lock().unwrap().insert("master".into(), sid);
        *surface.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "quiescing".into(),
            context_pct: None,
            task: None,
            updated_at: now(),
        });
        assert!(deliverable_master(&d).is_none(), "quiescing master는 배달 불가");
        // inbound → queued(주입 보류).
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hi", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("queued"), "quiescing 중 주입 보류: {r}");
    }

    /// C1(가)-1: quiescing 중엔 보류(queued), 해제(비-quiescing 자기보고) 후엔 inbox가 드레인되어
    /// 주입(injected)된다 — cycle-agent가 clear 전 quiescing 설정·resume 후 해제하는 계약의 봉합.
    #[cfg(unix)]
    #[test]
    fn quiescing_release_then_drains() {
        let d = tmp_daemon("quiesce_drain");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let surface = d
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("surface");
        let sid = surface.id;
        d.roles.lock().unwrap().insert("master".into(), sid);
        // quiescing 자기보고 → 주입 보류.
        *surface.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "quiescing".into(),
            context_pct: None,
            task: None,
            updated_at: now(),
        });
        let r = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hi", "user"), own_pid());
        assert_eq!(r["result"]["action"], json!("queued"), "quiescing 중 보류: {r}");
        let inbox_id = r["result"]["inbox_id"].as_i64().unwrap();
        // quiescing 해제(cycle-agent resume) → deliver → injected.
        *surface.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "working".into(),
            context_pct: None,
            task: None,
            updated_at: now(),
        });
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            let delivered = deliver_new_inbox(&d, conn);
            assert!(delivered.contains(&inbox_id), "해제 후 drain 배달: {delivered:?}");
            let st: String = conn
                .query_row("SELECT state FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
                .unwrap();
            assert_eq!(st, "injected");
        }
        let _ = crate::governance::close_surface(&d, sid, crate::governance::CloseCause::Reap);
    }

    /// MED-3: master가 배달 중 quiescing으로 전환되면 이후 주입은 try_send 없이 보류(false)된다.
    /// 루프 상단 deliverable_master 게이트를 통과한 뒤 mid-loop로 quiescing set되는 TOCTOU 창을
    /// inject_master의 주입-직전 재확인이 봉합함을 직접 단위검증(inject_master==false).
    #[cfg(unix)]
    #[test]
    fn inject_master_reblocks_on_midloop_quiescing() {
        let d = tmp_daemon("inject_recheck");
        let surface = d
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("surface");
        let sid = surface.id;
        d.roles.lock().unwrap().insert("master".into(), sid);
        // 비-quiescing(agent_status None) → 주입 가능.
        assert_eq!(deliverable_master(&d), Some(sid), "비-quiescing master는 배달 가능");
        assert!(inject_master(&d, sid, "[test] hi"), "비-quiescing master엔 주입 성공");
        // 루프 중 quiescing set(cycle-agent가 /clear 진입 직전) → 주입 직전 재확인이 false 반환.
        *surface.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "quiescing".into(),
            context_pct: None,
            task: None,
            updated_at: now(),
        });
        assert_eq!(deliverable_master(&d), None, "quiescing master는 배달 불가");
        assert!(!inject_master(&d, sid, "[test] hi2"), "quiescing surface엔 주입 보류(false)");
        let _ = crate::governance::close_surface(&d, sid, crate::governance::CloseCause::Reap);
    }

    /// C1(가)-2: pause 중엔 (a) inbound가 master 가용해도 queued(주입 동결)·inbox 적재는 지속(유실 0)
    /// (b) outbound 행은 pending 생성되나 channel.outbound.<ch> 이벤트는 미발행. resume 후 resume_flush가
    /// (a) 보류 inbox를 드레인 (b) 동결 outbound를 재발행한다(§2.6 O5).
    #[cfg(unix)]
    #[test]
    fn pause_freezes_outbound_event_and_inbox_then_resume_flush() {
        use std::sync::atomic::Ordering;
        let d = tmp_daemon("pause");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // 정상이라면 즉시 주입될 가용·비-quiescing master surface.
        let surface = d
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("surface");
        let sid = surface.id;
        d.roles.lock().unwrap().insert("master".into(), sid);
        let mut rx = d.bus.subscribe();
        // ── pause 진입 ──
        d.paused.store(true, Ordering::Relaxed);
        // 인바운드: master 가용해도 pause라 queued(주입 동결)·적재는 지속.
        let inb = call(&d, "inbound", inbound_params("slack", "U1", "slack:1", "hi", "user"), own_pid());
        assert_eq!(inb["result"]["action"], json!("queued"), "pause 중 주입 동결→queued: {inb}");
        let inbox_id = inb["result"]["inbox_id"].as_i64().unwrap();
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let st: String = conn
                .query_row("SELECT state FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
                .unwrap();
            assert_eq!(st, "new", "pause 중에도 inbox 적재는 지속(유실 0)");
        }
        // 아웃바운드: 행은 pending, 이벤트는 미발행.
        let ob = call(&d, "outbound", json!({"channel": "slack", "target": "U1", "body": "x", "idempotency_key": "o1"}), None);
        assert_eq!(ob["result"]["outcome"], json!("pending"));
        let mut saw_outbound_evt = false;
        while let Ok(ev) = rx.try_recv() {
            if ev["name"].as_str() == Some("channel.outbound.slack") {
                saw_outbound_evt = true;
            }
        }
        assert!(!saw_outbound_evt, "pause 중 outbound 이벤트가 발행되면 안 된다");
        // ── resume ──
        d.paused.store(false, Ordering::Relaxed);
        resume_flush(&d);
        let mut republished = false;
        while let Ok(ev) = rx.try_recv() {
            if ev["name"].as_str() == Some("channel.outbound.slack")
                && ev["payload"]["idempotency_key"].as_str() == Some("o1")
            {
                republished = true;
            }
        }
        assert!(republished, "resume_flush가 동결된 outbound를 재발행해야 한다");
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let st: String = conn
                .query_row("SELECT state FROM inbox WHERE id=?1", [inbox_id], |r| r.get(0))
                .unwrap();
            assert_eq!(st, "injected", "resume 후 보류 inbox가 드레인되어 주입돼야 한다");
        }
        let _ = crate::governance::close_surface(&d, sid, crate::governance::CloseCause::Reap);
    }

    /// C1(가)-2 보강: pause 중엔 sweep의 outbound 타임아웃 시계도 동결된다 — 배달을 얼린 동안
    /// pending을 unknown(terminal)으로 넘기면 resume_flush(pending만 재발행)가 항목을 잃기 때문.
    #[test]
    fn pause_freezes_outbound_timeout_clock() {
        use std::sync::atomic::Ordering;
        let d = tmp_daemon("pause_to");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        let a = call(&d, "outbound", json!({"channel":"slack","target":"U1","body":"x","idempotency_key":"o1"}), None);
        let oid = a["result"]["outbound_id"].as_i64().unwrap();
        // created_ts를 타임아웃 창보다 훨씬 과거로 → 정상이라면 sweep이 unknown 전이.
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            conn.execute("UPDATE outbound SET created_ts=?2 WHERE id=?1", params![oid, now() - 100000.0]).unwrap();
        }
        // pause 중 sweep → 타임아웃 동결(pending 유지).
        d.paused.store(true, Ordering::Relaxed);
        sweep_once(&d);
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let oc: String = conn.query_row("SELECT outcome FROM outbound WHERE id=?1", [oid], |r| r.get(0)).unwrap();
            assert_eq!(oc, "pending", "pause 중엔 타임아웃 동결(pending 유지)");
        }
        // resume 후 sweep → 타임아웃 시계 재개 → unknown 전이.
        d.paused.store(false, Ordering::Relaxed);
        sweep_once(&d);
        {
            let g = d.channels.lock().unwrap();
            let conn = g.as_ref().unwrap();
            let oc: String = conn.query_row("SELECT outcome FROM outbound WHERE id=?1", [oid], |r| r.get(0)).unwrap();
            assert_eq!(oc, "unknown", "resume 후엔 타임아웃 시계가 재개되어 unknown");
        }
    }

    // ── C2: feed tier 태깅·승인 미러·interaction 검증 ─────────────────────────────

    fn push_pending_feed(d: &Arc<Daemon>, request_id: &str) {
        d.feed_items.lock().unwrap().push(crate::state::FeedItem {
            request_id: request_id.into(),
            kind: "permission".into(),
            title: "approval".into(),
            body: "body".into(),
            surface_id: None,
            status: "pending".into(),
            decision: None,
            created_at: now(),
            resolved_at: None,
            tier: Some("c".into()),
            publisher_pid: None,
            publisher_pgid: None,
            publisher_surface: None,
            risk_class: None,
            auto_route: false,
        });
    }

    fn mirror_nonce(d: &Arc<Daemon>, channel: &str, feed_id: &str, target: &str) -> Option<String> {
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        conn.query_row(
            "SELECT approval_nonce FROM outbound WHERE channel=?1 AND approval_feed_id=?2 AND target=?3 AND kind='approval_prompt'",
            params![channel, feed_id, target],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    fn count_approval_prompts(d: &Arc<Daemon>, channel: &str, feed_id: &str) -> i64 {
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM outbound WHERE channel=?1 AND approval_feed_id=?2 AND kind='approval_prompt'",
            params![channel, feed_id],
            |r| r.get(0),
        )
        .unwrap_or(-1)
    }

    fn interaction_params(channel: &str, sender: &str, feed_id: &str, nonce: &str, decision: &str) -> Value {
        json!({"channel": channel, "sender_id": sender,
               "idempotency_key": format!("{channel}:interaction:{feed_id}:{nonce}"),
               "kind": "interaction", "feed_id": feed_id, "nonce": nonce, "decision": decision})
    }

    #[test]
    fn tier_mirrorable_pure() {
        assert!(tier_mirrorable(Some("a")));
        assert!(tier_mirrorable(Some("b")));
        assert!(tier_mirrorable(Some("c")));
        assert!(!tier_mirrorable(Some("d")), "Tier D는 절대 미러 금지");
        assert!(!tier_mirrorable(None), "무태그=D 취급(fail-closed)");
        assert!(!tier_mirrorable(Some("x")), "미지 tier도 fail-closed");
    }

    #[test]
    fn schema_version_is_3() {
        let d = tmp_daemon("schema3");
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let v: String = conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "3", "OPP-21 v3(원격승인 상한·쿨다운)");
        // approval_nonce_used 컬럼 존재(v2 마이그레이션 확인).
        let has: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('outbound') WHERE name='approval_nonce_used'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has, 1, "approval_nonce_used 컬럼이 있어야 한다");
        // v3 신규 테이블 존재(OPP-21 마이그레이션 확인).
        for t in ["approval_attempts", "remote_approve_daily"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "v3 테이블 {t} 존재해야");
        }
    }

    #[test]
    fn mirror_blocked_when_tier_d_or_untagged() {
        let d = tmp_daemon("mirror_faild");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None); // 게이트 ON
        // tier=d → 미러 금지.
        mirror_approval(&d, "feedD", "t", "b", Some("d"));
        assert_eq!(count_approval_prompts(&d, "slack", "feedD"), 0, "Tier D는 미러되면 안 된다");
        // 무태그(None) → 미러 금지(fail-closed).
        mirror_approval(&d, "feedNone", "t", "b", None);
        assert_eq!(count_approval_prompts(&d, "slack", "feedNone"), 0, "무태그는 미러되면 안 된다");
    }

    #[test]
    fn mirror_blocked_when_gate_off() {
        let d = tmp_daemon("mirror_gateoff");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // 게이트 OFF(기본) — tier=c여도 미러 금지.
        mirror_approval(&d, "feedC", "t", "b", Some("c"));
        assert_eq!(count_approval_prompts(&d, "slack", "feedC"), 0, "게이트 OFF면 미러 금지");
        // 게이트 ON 후엔 미러됨.
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        mirror_approval(&d, "feedC", "t", "b", Some("c"));
        assert_eq!(count_approval_prompts(&d, "slack", "feedC"), 1, "게이트 ON·tier≤C면 미러 1건");
    }

    #[test]
    fn mirror_idempotent_no_duplicate_button() {
        let d = tmp_daemon("mirror_idem");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        mirror_approval(&d, "feedX", "t", "b", Some("c"));
        // 재호출(aging 모사) → 신규 outbound 발행 없음(중복 버튼 금지·O9).
        mirror_approval(&d, "feedX", "t", "b", Some("c"));
        assert_eq!(count_approval_prompts(&d, "slack", "feedX"), 1, "aging 재미러는 버튼 1건 유지");
    }

    #[test]
    fn status_reports_remote_approve() {
        let d = tmp_daemon("status_ra");
        seed_registered(&d, "slack", "t");
        let s0 = call(&d, "status", json!({}), None);
        assert_eq!(s0["result"]["remote_approve"]["active"], json!(false), "기본 OFF");
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        let s1 = call(&d, "status", json!({}), None);
        assert_eq!(s1["result"]["remote_approve"]["active"], json!(true), "opt-in 후 ON");
        // --off(duration_secs=0) → 다시 OFF.
        call(&d, "allow-remote-approve", json!({"duration_secs": 0}), None);
        let s2 = call(&d, "status", json!({}), None);
        assert_eq!(s2["result"]["remote_approve"]["active"], json!(false), "--off 후 OFF");
    }

    #[test]
    fn interaction_valid_button_resolves_feed() {
        let d = tmp_daemon("intr_ok");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").expect("nonce minted");
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("approval_resolved"), "{r}");
        assert_eq!(r["result"]["decision"], json!("allow"));
        // feed가 resolved.
        let resolved = d
            .feed_items
            .lock()
            .unwrap()
            .iter()
            .any(|i| i.request_id == "feed1" && i.status == "resolved" && i.decision.as_deref() == Some("allow"));
        assert!(resolved, "interaction이 feed를 resolved로 해소해야 한다");
    }

    #[test]
    fn interaction_replay_rejected_nonce_burned() {
        let d = tmp_daemon("intr_replay");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        let ok = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(ok["result"]["action"], json!("approval_resolved"));
        // 재생(같은 nonce 재사용) → 소각됨 → denied(nonce_used).
        push_pending_feed(&d, "feed1b"); // (feed는 이미 resolved라 상관없이) nonce가 먼저 막는다
        let replay = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(replay["result"]["action"], json!("interaction_denied"), "{replay}");
        assert_eq!(replay["result"]["reason"], json!("nonce_used"), "재생은 nonce_used로 거부");
    }

    #[test]
    fn interaction_forged_nonce_rejected() {
        let d = tmp_daemon("intr_forge");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", "deadbeefforged", "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("nonce_invalid"), "위조 nonce는 nonce_invalid");
    }

    #[test]
    fn interaction_non_owner_rejected() {
        let d = tmp_daemon("intr_nonowner");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        // 비-allowlist sender가 유효 nonce로 시도 → not_owner.
        let r = call(&d, "inbound", interaction_params("slack", "U_evil", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("not_owner"), "비owner는 not_owner");
    }

    #[test]
    fn interaction_gate_off_rejected() {
        let d = tmp_daemon("intr_gateoff");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        // 게이트 ON 상태로 미러 발행해 nonce 확보.
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        // 게이트 닫기(--off) → 유효 nonce·owner여도 승인 처리 금지.
        call(&d, "allow-remote-approve", json!({"duration_secs": 0}), None);
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("remote_approve_off"), "게이트 OFF면 승인 처리 금지");
    }

    #[test]
    fn interaction_owner_mismatch_rejected() {
        // 다른 owner용으로 발행된 nonce를 또 다른 allowlist owner가 도용 → owner_mismatch.
        let d = tmp_daemon("intr_ownermis");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U2"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        // U1용 nonce를 U2가 사용 시도.
        let nonce_u1 = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        let r = call(&d, "inbound", interaction_params("slack", "U2", "feed1", &nonce_u1, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("owner_mismatch"), "타 owner nonce 도용은 owner_mismatch");
    }

    #[test]
    fn interaction_unknown_feed_rejected() {
        // 유효 nonce·owner·게이트 ON이지만 feed_id가 pending 목록에 없음(미존재/미지) → feed_not_pending.
        let d = tmp_daemon("intr_unknownfeed");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        // feed를 pending으로 push하지 않은 채 미러만 발행(nonce 확보) → feed는 미존재.
        mirror_approval(&d, "ghostfeed", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "ghostfeed", "U1").unwrap();
        let r = call(&d, "inbound", interaction_params("slack", "U1", "ghostfeed", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("feed_not_pending"), "미지 feed_id는 feed_not_pending");
    }

    #[test]
    fn interaction_expired_rejected() {
        // opt-in 기간 만료 후엔 유효 nonce·owner여도 승인 처리 금지(remote_approve_off).
        let d = tmp_daemon("intr_expired");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        // 자연 만료 모사: 만료 ts를 과거로 민다(remote_approve_active = until > now → false).
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            meta_set(conn, "allow_remote_approve_until", &(now() - 10.0).to_string());
        }
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("remote_approve_off"), "만료 후는 remote_approve_off");
    }

    #[test]
    fn mirror_blocked_after_expiry() {
        // opt-in 만료 후 발행되는 feed는 미러되지 않는다(미러 게이트가 만료를 재확인).
        let d = tmp_daemon("mirror_expired");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        {
            let mut g = d.channels.lock().unwrap();
            let conn = g.as_mut().unwrap();
            meta_set(conn, "allow_remote_approve_until", &(now() - 10.0).to_string());
        }
        mirror_approval(&d, "feedLate", "t", "b", Some("c"));
        assert_eq!(count_approval_prompts(&d, "slack", "feedLate"), 0, "만료 후엔 미러 금지");
    }

    // ── OPP-21 §5 자기공격 변이 테스트(V4·V6·V7 신규 · V2 누락분 보강) ──────────────

    fn seed_daily_allow(d: &Arc<Daemon>, count: i64) {
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        let day = local_date(now());
        conn.execute(
            "INSERT INTO remote_approve_daily(day, allow_count, updated_ts) VALUES(?1,?2,?3)
             ON CONFLICT(day) DO UPDATE SET allow_count=?2, updated_ts=?3",
            params![day, count, now()],
        )
        .unwrap();
    }

    fn read_daily(d: &Arc<Daemon>) -> u64 {
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        daily_allow_count(conn, &local_date(now()))
    }

    fn nonce_used_flag(d: &Arc<Daemon>, channel: &str, nonce: &str) -> i64 {
        let g = d.channels.lock().unwrap();
        let conn = g.as_ref().unwrap();
        conn.query_row(
            "SELECT approval_nonce_used FROM outbound WHERE channel=?1 AND approval_nonce=?2",
            params![channel, nonce],
            |r| r.get(0),
        )
        .unwrap_or(-1)
    }

    fn feed_status(d: &Arc<Daemon>, feed_id: &str) -> Option<String> {
        d.feed_items
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.request_id == feed_id)
            .map(|i| i.status.clone())
    }

    // ── V6: redact 필터(토큰·개인경로 스크럽) ──
    #[test]
    fn v6_redact_scrubs_tokens_and_paths_preserves_plain() {
        // 토큰·경로는 스크럽, 일반 단어는 보존.
        // secret-scan(공개 발행 fail-closed) 통과: 시크릿 형상을 조각으로 조립 —
        // 런타임 값은 동일(redact 검증 불변), 소스엔 연속 시크릿 패턴 미존재.
        let sf = |a: &str, b: &str| format!("{a}{b}");
        let home = sf("/Users/", "cys");
        let slack = sf("xoxb-", "123456789-abcdefghij");
        let (r, hit) = redact(&format!("token {slack} path {home}/secret/key.txt done"));
        assert!(hit, "토큰/경로 있으면 redact 발생");
        assert!(!r.contains(&sf("xoxb-", "123456789")), "Slack 토큰 스크럽: {r}");
        assert!(!r.contains(&home), "개인경로 스크럽: {r}");
        assert!(r.contains("[redacted]"), "치환 마커 존재: {r}");
        assert!(r.contains("token") && r.contains("path") && r.contains("done"), "일반 단어 보존: {r}");
        // 다양한 시크릿 형상.
        assert!(redact(&sf("sk-", "abcdefghijklmnop1234567")).1, "OpenAI sk- 스크럽");
        assert!(redact(&sf("ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")).1, "GitHub 토큰 스크럽");
        assert!(redact(&sf("AKIA", "IOSFODNN7EXAMPLE")).1, "AWS access key 스크럽");
        assert!(redact("deadbeefdeadbeefdeadbeefdeadbeef01").1, "장문 hex 스크럽");
        // 짧은/평범한 문자열·한국어는 보존(오탐 방지).
        let (p, hitp) = redact("일반 승인 요청 배포 확인 abc123");
        assert!(!hitp, "평범한 문자열은 redact 없음");
        assert_eq!(p, "일반 승인 요청 배포 확인 abc123");
    }

    #[test]
    fn v6_mirror_body_redacts_and_points_local() {
        // 카드 조립 경로가 redact를 경유하는지(V6 배선).
        let sf = |a: &str, b: &str| format!("{a}{b}");
        let home = sf("/Users/", "cys");
        let tok = sf("xoxb-", "999888777-secrettoken");
        let body = mirror_body(&format!("배포 {tok}"), &format!("경로 {home}/.env 유출 위험"), "feedZ", now());
        assert!(!body.contains(&sf("xoxb-", "999888777")), "카드에 토큰 유출 금지: {body}");
        assert!(!body.contains(&home), "카드에 개인경로 유출 금지: {body}");
        assert!(body.contains("[redacted]"), "가림 마커: {body}");
        assert!(body.contains("민감정보 가림"), "원문=로컬 포인터 명시: {body}");
        assert!(body.contains("feed:feedZ"), "feed 포인터 유지: {body}");
        // 민감정보 없으면 note 미부착(외과적).
        let clean = mirror_body("일반 배포 승인", "스테이징 반영", "feedC", now());
        assert!(!clean.contains("민감정보 가림"), "클린 본문엔 가림 note 없음: {clean}");
    }

    // ── V7: 일일 원격 allow 상한 ──
    #[test]
    fn v7_daily_cap_21st_allow_rejected_pending_preserved() {
        let d = tmp_daemon("v7_cap");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        // 오늘 이미 상한(20) 소진 → 21건째 allow는 거부.
        seed_daily_allow(&d, REMOTE_APPROVE_DAILY_CAP as i64);
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("daily_cap"), "상한 초과는 daily_cap");
        // 소각 前 거부이므로 nonce 미소각·feed pending 유지(로컬 처리 가능).
        assert_eq!(nonce_used_flag(&d, "slack", &nonce), 0, "상한 거부는 nonce 미소각");
        assert_eq!(feed_status(&d, "feed1").as_deref(), Some("pending"), "상한 거부는 feed pending 유지");
    }

    #[test]
    fn v7_under_cap_allow_succeeds_and_increments() {
        let d = tmp_daemon("v7_under");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        seed_daily_allow(&d, (REMOTE_APPROVE_DAILY_CAP - 1) as i64); // 19 → 20건째는 통과.
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("approval_resolved"), "{r}");
        assert_eq!(read_daily(&d), REMOTE_APPROVE_DAILY_CAP, "성공 allow는 상한 카운터 +1(19→20)");
    }

    #[test]
    fn v7_cap_counts_allow_not_deny() {
        // 상한 소진 상태여도 deny 해소는 허용(안전측)·카운터 불변.
        let d = tmp_daemon("v7_deny");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        seed_daily_allow(&d, REMOTE_APPROVE_DAILY_CAP as i64); // 상한 도달.
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &nonce, "deny"), own_pid());
        assert_eq!(r["result"]["action"], json!("approval_resolved"), "deny는 상한과 무관 해소: {r}");
        assert_eq!(r["result"]["decision"], json!("deny"));
        assert_eq!(read_daily(&d), REMOTE_APPROVE_DAILY_CAP, "deny 해소는 allow 카운터 불변");
    }

    // ── V4: 연속 실패 sender 쿨다운 ──
    #[test]
    fn v4_sender_cooldown_after_consecutive_fails() {
        let d = tmp_daemon("v4_cool");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U2"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        // U1이 위조 nonce로 임계(5)회 연속 실패 → 브루트.
        for i in 0..SENDER_COOLDOWN_FAIL_THRESHOLD {
            let bogus = format!("forged{i}");
            let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &bogus, "allow"), own_pid());
            assert_eq!(r["result"]["reason"], json!("nonce_invalid"), "브루트 {i}회차는 nonce_invalid: {r}");
        }
        // 6번째: 유효 nonce여도 쿨다운으로 거부(가장 앞선 게이트).
        let good = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &good, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("sender_cooldown"), "임계 초과 sender는 쿨다운");
        // feed 미해소(쿨다운이 유효 승인도 막음).
        assert_eq!(feed_status(&d, "feed1").as_deref(), Some("pending"), "쿨다운 중 feed pending 유지");
        // 다른 sender(U2)는 영향 없음(per-sender 쿨다운).
        let r2 = call(&d, "inbound", interaction_params("slack", "U2", "feed1", "forgedX", "allow"), own_pid());
        assert_eq!(r2["result"]["reason"], json!("nonce_invalid"), "U2는 쿨다운 무관(신선): {r2}");
    }

    #[test]
    fn v4_success_resets_consecutive_fails() {
        let d = tmp_daemon("v4_reset");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        // 임계 미만(4)회 실패.
        for i in 0..(SENDER_COOLDOWN_FAIL_THRESHOLD - 1) {
            let bogus = format!("forged{i}");
            call(&d, "inbound", interaction_params("slack", "U1", "feed1", &bogus, "allow"), own_pid());
        }
        // 유효 승인 성공 → 카운터 리셋.
        let good = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        let ok = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &good, "allow"), own_pid());
        assert_eq!(ok["result"]["action"], json!("approval_resolved"), "{ok}");
        // 리셋 후: 다시 임계-1회 실패해도 쿨다운 미발동(리셋 안 됐다면 4+1=5로 이미 쿨다운).
        for i in 0..(SENDER_COOLDOWN_FAIL_THRESHOLD - 1) {
            let bogus = format!("post{i}");
            let r = call(&d, "inbound", interaction_params("slack", "U1", "feed1", &bogus, "allow"), own_pid());
            assert_eq!(r["result"]["reason"], json!("nonce_invalid"), "리셋 후 {i}회차는 nonce_invalid(쿨다운 아님): {r}");
        }
    }

    // ── V2 보강: nonce_feed_mismatch(기존 owner_mismatch만 명명돼 누락분 박제) ──
    #[test]
    fn v2_nonce_feed_mismatch_rejected() {
        let d = tmp_daemon("v2_feedmis");
        seed_registered(&d, "slack", "t");
        call(&d, "allow", json!({"channel": "slack", "sender_id": "U1"}), None);
        call(&d, "allow-remote-approve", json!({"duration_secs": 3600}), None);
        push_pending_feed(&d, "feed1");
        mirror_approval(&d, "feed1", "t", "b", Some("c"));
        let nonce = mirror_nonce(&d, "slack", "feed1", "U1").unwrap();
        // feed1에 결박된 nonce를 다른 feed_id로 제시(재생/오결박) → nonce_feed_mismatch.
        let r = call(&d, "inbound", interaction_params("slack", "U1", "feedOTHER", &nonce, "allow"), own_pid());
        assert_eq!(r["result"]["action"], json!("interaction_denied"), "{r}");
        assert_eq!(r["result"]["reason"], json!("nonce_feed_mismatch"), "타 feed 결박 nonce는 mismatch");
    }
}
