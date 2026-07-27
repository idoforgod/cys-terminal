//! T5 사용량 관측 수집기 — 에이전트 CLI의 로컬 산출물을 무간섭(passive) 관측해
//! context 사용량·rate limit 잔량을 결정론 산출한다. `cys set-status` 자기보고(LLM 추론)의
//! 관측 보강 — 절대지침 "결정론 환원"의 사용량 축.
//!
//! 데이터 소스 (실측 검증 2026-06-13):
//! - claude: `~/.claude*/projects/<munged-cwd>/<session>.jsonl` — assistant 라인의
//!   `message.usage`. 현재 컨텍스트 = input + cache_read + cache_creation (output 제외 —
//!   공식 statusline 문서의 used_percentage 공식과 동일). `isSidechain:true`(서브에이전트)
//!   라인은 메인 컨텍스트가 아니므로 제외. rate limit은 로컬 파일에 없음(Phase 2 statusline).
//! - codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` — `token_count` 이벤트의
//!   `info.last_token_usage`(컨텍스트)·`model_context_window`·`rate_limits`(primary 5h /
//!   secondary 7d, used_percent·resets_at).
//! - gemini(agy): 토큰·쿼터를 평문 로컬 파일에 남기지 않음 — Phase 2(로컬 RPC) 대상, 여기선 스킵.
//!
//! pane↔세션 매핑 우선순위:
//! ① `usage.register` RPC (SessionStart hook이 transcript_path를 등록 — 같은 cwd 동시
//!    세션 다수와 무관한 결정론 1:1)
//! ② codex: 에이전트 프로세스의 열린 fd(lsof)에서 rollout 경로 직독
//! ③ 휴리스틱 폴백: 에이전트 프로세스 cwd 기준 디렉터리에서 pane 생성 이후 mtime 최신 파일
//!    (동시 세션 경합 시 오귀속 가능 — usage.source로 구분 노출)
//!
//! 외부(비-pane) 세션: pane 밖 Claude Code 세션의 트랜스크립트도 주기 스윕으로 소비만
//! 적재한다(role="external[:프로필]") — collect_external 참조.

use crate::state::{now_epoch, Daemon, Surface};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 최초 attach 시 파일 끝에서 거슬러 읽는 창 (최신 usage 라인은 이 안에 있다)
const FIRST_ATTACH_TAIL: u64 = 256 * 1024;
/// 틱당 최대 읽기 — 초과분은 따라잡기를 포기하고 마지막 창으로 점프 (데몬 정체 방지)
const MAX_READ_PER_TICK: u64 = 4 * 1024 * 1024;
/// 미완성 라인 carry 상한 — 초과 시 폐기 (개행 없는 거대 라인의 메모리 무한 성장 차단)
const MAX_CARRY: usize = 8 * 1024 * 1024;
/// 휴리스틱(비등록) 매핑의 재발견 주기 초 — 새 세션 파일(/clear 등) 전환 추적
const REDISCOVER_SECS: f64 = 30.0;
/// statusline 보고(usage.report) 신선도 창 초 — claude는 이 안에 statusline 보고가 있으면
/// 트랜스크립트 tail이 ctx를 덮어써 rate limit을 유실시키지 않게 수집을 건너뛴다(우선순위 병합).
const STATUSLINE_FRESH_SECS: f64 = 60.0;
/// 외부(비-pane) 세션 스윕 주기 초 기본값 — CYS_USAGE_EXTERNAL_SECS로 조정(0=끔)
const EXTERNAL_SWEEP_SECS_DEFAULT: u64 = 15;
/// 외부 세션 추적 시작 조건: 이 창 안에 mtime이 있는 활동 파일만 (과거 세션 소급 적재 금지)
const EXTERNAL_ACTIVE_SECS: f64 = 600.0;

/// rate limit 윈도우 1개 (codex primary/secondary; Phase 2에서 claude 5h/7d 합류)
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RateWindow {
    pub label: String, // "5h" | "7d" | "Nm" | "?"
    pub used_pct: f64,
    pub resets_at: Option<f64>, // unix epoch 초
}

/// 관측 사용량 스냅샷 — Surface.observed_usage에 저장, surface.list/org.status로 노출
#[derive(Clone, Debug, serde::Serialize)]
pub struct ObservedUsage {
    pub agent: String,
    pub ctx_tokens: Option<u64>,
    pub ctx_window: Option<u64>,
    pub ctx_pct: Option<u8>,
    pub rate: Vec<RateWindow>,
    /// "transcript[:heuristic]"(claude tail) | "rollout[:heuristic]"(codex tail) |
    /// "statusline"(usage.report 서버 진실 — 신선하면 tail 관측보다 우선)
    pub source: String,
    pub session_file: String,
    pub updated_at: f64,
    /// ★CU-6A 귀속 신뢰도(additive · `"confident"|"ambiguous"|"evicted"`, 비클로드/구경로는 `None`).
    ///
    /// 왜 값을 지우지 않고 표식을 다는가: 오귀속이 확정돼도 스냅샷을 지우면 소비자는 "관측 없음"
    /// 으로 보고 **조용히** 자기보고로 되돌아간다. 틀렸다는 사실 자체가 소비자에게 필요한 신호다
    /// (표는 관측 대신 자기보고를 그리고, 60% 임계 게이트는 이 값을 보고 미투입한다).
    /// `None`은 "판정 안 함"이며 **종전 동작**을 뜻한다 — 게이트는 `None`을 통과시킨다(회귀 0).
    pub attribution: Option<String>,
}

/// surface별 tail 진행 상태 (수집기 태스크 로컬 — 데몬 상태 오염 없음)
struct TailState {
    path: PathBuf,
    offset: u64,
    carry: String,
    /// 휴리스틱 매핑 여부 — true면 REDISCOVER_SECS마다 재발견 (등록 매핑은 고정)
    heuristic: bool,
    last_discovery: f64,
    /// statusline이 준 서버 진실 컨텍스트 창 — statusline이 끊긴 뒤 트랜스크립트 폴백의
    /// 200k 하드코딩 추정(1M 세션 5배 과대→임계 조기오발)을 교정한다(전수조사 B-5).
    server_ctx_window: Option<u64>,
    /// codex rollout의 turn_context가 준 모델명 — token_count 소비 귀속용(전수조사 A-2)
    codex_model: Option<String>,
    /// ★CU-6A 이 매핑의 **신뢰 등급**(선점 서열의 근거). `heuristic`과 별개인 이유: `heuristic`은
    /// "주기적으로 재발견할 것인가"라는 수명 정책이고, rank는 "이 매핑을 얼마나 믿는가"다.
    /// codex lsof 매핑은 결정론(rank 2)이면서도 /clear 후 새 rollout 추적을 위해 재발견 대상
    /// (heuristic=true)이라, 둘을 한 불리언으로 겸하면 반드시 한쪽이 틀린다.
    rank: u8,
}

impl TailState {
    /// 새 tail — 영속 오프셋(analytics tail_offsets)이 있으면 거기서 정확 재개해
    /// 재시작 시 마지막 256KB 재파싱→DB 중복 INSERT(전수조사 A-4)를 근절한다.
    fn attach(daemon: &Arc<Daemon>, path: PathBuf, heuristic: bool, rank: u8, now: f64) -> Self {
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let stored = daemon
            .analytics
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|c| crate::analytics::load_offset(c, &path.to_string_lossy()));
        let offset = match stored {
            Some(o) if o <= len => o,
            _ => len.saturating_sub(FIRST_ATTACH_TAIL),
        };
        TailState { path, offset, carry: String::new(), heuristic, last_discovery: now, server_ctx_window: None, codex_model: None, rank }
    }
}

fn poll_secs() -> u64 {
    cys::env_compat("CYS_USAGE_POLL_SECS")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(2)
}

pub fn spawn_usage_collector(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut tails: HashMap<u64, TailState> = HashMap::new();
        let mut attempts: HashMap<u64, f64> = HashMap::new();
        let mut ext = ExternalTails::default();
        loop {
            tokio::time::sleep(Duration::from_secs(poll_secs())).await;
            // 패닉 격리 — watchdog과 동일: 한 틱의 패닉이 수집기를 영구 침묵시키지 않게
            let tick = std::panic::AssertUnwindSafe(|| {
                collect_tick(&daemon, &mut tails, &mut attempts, &mut ext)
            });
            if std::panic::catch_unwind(tick).is_err() {
                daemon.bus.publish(
                    "usage.tick_panic",
                    "usage",
                    None,
                    json!({"note": "usage collector tick panicked; continuing next tick"}),
                );
            }
        }
    });
}

fn collect_tick(
    daemon: &Arc<Daemon>,
    tails: &mut HashMap<u64, TailState>,
    attempts: &mut HashMap<u64, f64>,
    ext: &mut ExternalTails,
) {
    let surfaces: Vec<Arc<Surface>> = daemon.surfaces.lock().unwrap().values().cloned().collect();
    let live_ids: HashSet<u64> = surfaces
        .iter()
        .filter(|s| !s.exited.load(Ordering::Relaxed))
        .map(|s| s.id)
        .collect();
    tails.retain(|sid, _| live_ids.contains(sid));
    attempts.retain(|sid, _| live_ids.contains(sid));
    // ★CU-6A 선점 유지보수 ① 죽은 pane의 선점 해제 — 안 하면 종료된 pane이 세션 파일을 영영
    // 쥐고 있어 같은 디렉터리의 후속 pane이 자기 세션을 관측하지 못한다.
    {
        let mut claims = daemon.session_claims.lock().unwrap();
        retain_live_claims(&mut claims, &live_ids);
    }
    // ★CU-6A 선점 유지보수 ② statusline(rank3) 선점 등록. statusline은 **에이전트 자신이 보고한**
    // 세션 파일이라 최상위 권위다. 이 선점이 폴백 발견보다 **먼저** 서야 폴백이 그 파일을 집지
    // 못한다(SIM-2 (a)). 폴백이 먼저 쥐고 있었다면 여기서 탈환한다(SIM-2 (b) — 무효화 1회).
    let now = now_epoch();
    for s in &surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let reported = s.observed_usage.lock().unwrap().as_ref().and_then(|u| {
            (u.source == "statusline"
                && now - u.updated_at < STATUSLINE_FRESH_SECS
                && !u.session_file.is_empty())
            .then(|| PathBuf::from(&u.session_file))
        });
        let Some(path) = reported else {
            continue;
        };
        let outcome = {
            let mut claims = daemon.session_claims.lock().unwrap();
            claim_session(&mut claims, &path, s.id, RANK_STATUSLINE)
        };
        if let ClaimOutcome::Evicted { evicted } = outcome {
            apply_eviction(daemon, tails, attempts, evicted, s.id, &path);
        }
    }
    for s in &surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let Some((agent, bin)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        match agent.as_str() {
            "claude" => collect_for(daemon, s, "claude", &bin, tails, attempts),
            "codex" => collect_for(daemon, s, "codex", &bin, tails, attempts),
            // gemini(agy)·grok: 로컬 평문 산출물에 토큰 미기록 — Phase 2 (로컬 RPC) 대상
            _ => {}
        }
    }
    collect_external(daemon, ext, &surfaces, tails);
}

/// 단일 surface 수집: 세션 파일 결정 → 증분 read → 파싱 → 스냅샷 갱신 → 이벤트 발행
fn collect_for(
    daemon: &Arc<Daemon>,
    s: &Arc<Surface>,
    agent: &str,
    bin: &str,
    tails: &mut HashMap<u64, TailState>,
    attempts: &mut HashMap<u64, f64>,
) {
    let registered = s.registered_transcript.lock().unwrap().clone();
    let now = now_epoch();

    // T5 Phase 2-A 우선순위 병합 — claude는 statusline 보고(rate limit + 서버 진실 ctx)가
    // 신선하면 트랜스크립트 tail이 ctx만 덮어써 rate를 유실시키지 않도록 **관측 스냅샷만** 건너뛴다.
    // ★소비 적재(record_message/record_usage)는 statusline과 무관하게 계속 돈다 — 과거엔 여기서
    // 함수 전체를 return해 statusline 가동 pane의 비용 통계가 전면 누락됐다(전수조사 A-1 교정).
    let statusline_fresh = agent == "claude"
        && s.observed_usage.lock().unwrap().as_ref().is_some_and(|prev| {
            prev.source == "statusline" && now - prev.updated_at < STATUSLINE_FRESH_SECS
        });

    // ── 세션 파일 결정 (등록 > lsof > 휴리스틱) ──
    // ★CU-6A: 결과에 **rank**(매핑 신뢰도)를 함께 싣는다. 선점 서열과 귀속 표식이 전부 이 값에서
    // 파생되므로, 발견 지점(어떻게 찾았는지 아는 유일한 곳)에서 붙이는 것이 정직하다.
    let desired: Option<(PathBuf, bool, u8)> = if let Some(reg) = registered {
        // 등록 매핑(usage.register — SessionStart hook)은 결정론 1:1이다.
        Some((PathBuf::from(reg), false, RANK_DETERMINISTIC))
    } else {
        let need_discovery = match tails.get(&s.id) {
            None => true,
            Some(t) => {
                !t.path.exists() || (t.heuristic && now - t.last_discovery > REDISCOVER_SECS)
            }
        };
        let existing = || {
            tails
                .get(&s.id)
                .filter(|t| t.path.exists())
                .map(|t| (t.path.clone(), t.heuristic, t.rank))
        };
        if need_discovery {
            // 발견 백오프: 실패가 반복돼도 전수 프로세스 refresh·lsof는 주기당 1회만
            // (자원 거버넌스 — 트랜스크립트가 아직 없는 pane이 틱마다 비용 유발 금지).
            // 신생 pane(1분 미만)은 트랜스크립트 지연 생성이 흔해 5초로 단축(전수조사 C-9 —
            // 구 30초 고정은 세션 초반 최대 30초 미수집 창을 만들었다).
            let backoff = if now - s.created_at < 60.0 { 5.0 } else { REDISCOVER_SECS };
            let recently = attempts
                .get(&s.id)
                .map(|t| now - *t < backoff)
                .unwrap_or(false);
            if recently {
                existing()
            } else {
                attempts.insert(s.id, now);
                // ★CU-6A: 상위·동급 rank가 선점한 후보는 건너뛰고 다음 후보를 본다. 여기서
                // 읽기만 하는 이유는 아래 스냅샷 직전의 선점 전이 한 곳에 상태 변경을 모으기
                // 위해서다(두 곳에서 맵을 바꾸면 탈환의 출처를 추적할 수 없다).
                let candidates = discover_session_candidates(s, agent, bin);
                let picked = {
                    let claims = daemon.session_claims.lock().unwrap();
                    first_unblocked(&claims, s.id, candidates)
                };
                picked
                    .map(|(p, rank)| (p, true, rank))
                    .or_else(existing)
            }
        } else {
            existing()
        }
    };
    let Some((path, heuristic, rank)) = desired else {
        // 미발견 — 다음 재발견 시도까지 빈 상태 유지 (배지 없음이 정직한 표현)
        return;
    };

    // (4a) resume 핀: 발견한 transcript에서 session_id를 1회 stash (is_none 가드).
    // 한번 잡으면 고정 — mtime 흔들림·동일 cwd 동시세션의 오핀을 방어한다.
    if s.agent_session_id.lock().unwrap().is_none() {
        if let Some(sid) = extract_session_id(agent, &path) {
            *s.agent_session_id.lock().unwrap() = Some(sid);
        }
    }

    // tail 상태 초기화/전환: 경로가 바뀌었으면 영속 오프셋(없으면 파일 끝 창)에서 새로 시작
    let need_reset = tails.get(&s.id).map(|t| t.path != path).unwrap_or(true);
    if need_reset {
        tails.insert(s.id, TailState::attach(daemon, path.clone(), heuristic, rank, now));
        // 새 세션 파일 = 새 세션 — 에지 게이트 재무장. 직전 세션이 임계 위에서 끝났어도
        // 새 세션이 곧장 임계 이상으로 시작하면(거대 지침 재주입) 발화해야 한다.
        s.ctx_threshold_armed.store(true, Ordering::Relaxed);
    } else if let Some(t) = tails.get_mut(&s.id) {
        t.heuristic = heuristic;
        // rank도 함께 최신화한다 — 같은 파일이 lsof로 재확인되면 결정론으로 승격될 수 있다.
        t.rank = rank;
        if heuristic {
            t.last_discovery = now;
        }
    }
    let Some(state) = tails.get_mut(&s.id) else {
        return;
    };

    // ── 증분 read + 파싱 (마지막 유효 관측이 승리) ──
    let lines = read_new_lines(state);
    if lines.is_empty() {
        return;
    }
    let prev = s.observed_usage.lock().unwrap().clone();
    // 서버 진실 컨텍스트 창 기억 — statusline이 살아있는 동안 준 ctx_window를 보관해
    // 폴백 시 200k 하드코딩 대신 사용(B-5). 한 번 잡히면 세션 내 고정.
    if let Some(p) = prev.as_ref() {
        if p.source == "statusline" && p.ctx_window.is_some() {
            state.server_ctx_window = p.ctx_window;
        }
    }
    let mut next: Option<ObservedUsage> = None;
    // CC v2 WS-A: 이 틱에 **신선 생산된** rate만 계정 귀속(claude transcript의 rate 이월분은
    // 제외 — 이월은 stale을 최신으로 둔갑시킨다. accounts.rs 모듈 헤더 계약).
    let mut codex_fresh_rate: Option<Vec<RateWindow>> = None;
    for line in &lines {
        match agent {
            "claude" => {
                if let Some((ctx_tokens, model)) = parse_claude_line(line) {
                    let window = state.server_ctx_window.unwrap_or_else(|| claude_ctx_window(&model));
                    next = Some(ObservedUsage {
                        agent: agent.into(),
                        ctx_tokens: Some(ctx_tokens),
                        ctx_window: Some(window),
                        ctx_pct: pct(ctx_tokens, window),
                        rate: next
                            .as_ref()
                            .map(|n| n.rate.clone())
                            .or_else(|| prev.as_ref().map(|p| p.rate.clone()))
                            .unwrap_or_default(),
                        source: source_label("transcript", state.heuristic),
                        session_file: state.path.to_string_lossy().into_owned(),
                        updated_at: now,
                        // 귀속 판정은 선점 전이 이후에 확정된다(아래) — 여기선 미판정.
                        attribution: None,
                    });
                }
            }
            "codex" => {
                if let Some(obs) = parse_codex_line(line) {
                    if let Some(fresh) = obs.rate.as_ref() {
                        codex_fresh_rate = Some(fresh.clone());
                    }
                    // 필드별 병합: token_count 이벤트에 info/rate_limits가 따로 올 수 있다
                    let base = next.as_ref().or(prev.as_ref());
                    let ctx_tokens = obs.ctx_tokens.or(base.and_then(|b| b.ctx_tokens));
                    let ctx_window = obs.ctx_window.or(base.and_then(|b| b.ctx_window));
                    let rate = obs
                        .rate
                        .or_else(|| base.map(|b| b.rate.clone()))
                        .unwrap_or_default();
                    next = Some(ObservedUsage {
                        agent: agent.into(),
                        ctx_tokens,
                        ctx_window,
                        ctx_pct: ctx_tokens
                            .zip(ctx_window)
                            .and_then(|(t, w)| pct(t, w)),
                        rate,
                        source: source_label("rollout", state.heuristic),
                        session_file: state.path.to_string_lossy().into_owned(),
                        updated_at: now,
                        attribution: None,
                    });
                }
            }
            _ => {}
        }
    }

    // CC v2 WS-A: codex rollout이 이 틱에 실제 생산한 rate → 계정 귀속(이월분 제외 계약)
    if let Some(fr) = codex_fresh_rate.as_ref() {
        crate::accounts::note_rate(
            daemon, "codex", &state.path.to_string_lossy(), fr, "rollout", now,
        );
    }

    // T6 Control Center 소비 누적 — claude/codex 새 메시지(턴)의 소비를 데몬 트래커에 적재.
    // tail은 새 라인을 1회만 읽고 오프셋을 영속하므로 재시작에도 이중계수 없음(A-4).
    let msgs: Vec<MsgCost> = match agent {
        "claude" => lines.iter().filter_map(|l| parse_claude_message_cost(l)).collect(),
        // codex rollout: turn_context의 model(gpt-5.5 등)을 기억했다가 token_count의
        // last_token_usage(턴 소비)에 귀속한다(전수조사 A-2 — codex 비용 가시화).
        "codex" => {
            for l in &lines {
                if let Some(m) = parse_codex_model(l) {
                    state.codex_model = Some(m);
                }
            }
            let model = state.codex_model.clone().unwrap_or_default();
            lines
                .iter()
                .filter_map(|l| parse_codex_message_cost(l))
                .map(|mut m| {
                    m.model = model.clone();
                    m
                })
                .collect()
        }
        _ => Vec::new(),
    };
    if !msgs.is_empty() {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let sess = path.to_string_lossy().into_owned();
        // D3: role(조직 단위 tier) 캐싱 — consumption/analytics 락 잡기 전에 1회(데드락 회피).
        // s.role은 Option<String> — None(미부여 노드)은 ""로 환원, summarize가 "unattributed"로 정규화.
        let role = s.role.lock().unwrap().clone().unwrap_or_default();
        let mut c = daemon.consumption.lock().unwrap();
        let alog = daemon.analytics.lock().unwrap(); // 일관 락 순서: consumption→analytics
        for m in msgs {
            let cost = crate::cost::calculate_cost(
                m.input_tokens, m.output, m.cache_creation, m.cache_read, &m.model,
            );
            // 소비 토큰 = input + cache_creation(+output) — cache_read(재사용)는 제외.
            c.record_message(
                &sess, m.input_tokens + m.cache_creation, m.output, cost, &m.model, now, &today,
            );
            // T7 E1-3: 영속 — 재시작에도 보존(부트 시 리플레이). 실패는 무해.
            if let Some(conn) = alog.as_ref() {
                crate::analytics::record_usage(
                    conn, &sess, &role, agent, &m.model, m.input_tokens, m.output,
                    m.cache_creation, m.cache_read, cost, now,
                );
            }
        }
        // 오프셋 영속 — 여기까지의 라인은 DB에 반영 완료. 재시작 시 이 지점에서 정확 재개(A-4).
        if let Some(conn) = alog.as_ref() {
            crate::analytics::save_offset(conn, &sess, state.offset, now);
        }
    }

    // statusline이 신선하면 관측 스냅샷·이벤트·임계발화는 statusline 경로가 진실원 — 여기서 종료
    // (소비 적재는 위에서 이미 완료). 끊기면(60s+) 아래 트랜스크립트 관측으로 graceful 폴백.
    if statusline_fresh {
        return;
    }

    let Some(mut new) = next else {
        return;
    };

    // ── ★CU-6A 선점 전이 (스냅샷 직전 · 상태 변경은 이 한 곳에서만) ──
    let outcome = {
        let mut claims = daemon.session_claims.lock().unwrap();
        claim_session(&mut claims, &path, s.id, rank)
    };
    match outcome {
        ClaimOutcome::Blocked { .. } => {
            // 상위(또는 동급 선착) rank가 이 파일을 쥐고 있다 — 관측을 **포기**한다.
            // 배지가 없는 것이 남의 컨텍스트를 내 것으로 그리는 것보다 낫다. tail을 놓아
            // 다음 발견 창에서 다른 후보를 찾게 한다(굳어 버리는 것을 막는다).
            tails.remove(&s.id);
            return;
        }
        ClaimOutcome::Evicted { evicted } => {
            apply_eviction(daemon, tails, attempts, evicted, s.id, &path)
        }
        ClaimOutcome::Granted => {}
    }

    // ★SIM-2 (c): 선점만으로는 부족하다 — 밀려난 pane은 "다음 후보"로 옮겨가 **또 다른 남의
    // 파일**을 집는다. 같은 프로젝트 디렉터리에 폴백 pane이 둘 이상이면 전원 모호로 내린다.
    let ambiguous = {
        let claims = daemon.session_claims.lock().unwrap();
        ambiguous_surfaces(&claims).contains(&s.id)
    };
    new.attribution = Some(
        if ambiguous {
            ATTR_AMBIGUOUS
        } else {
            ATTR_CONFIDENT
        }
        .into(),
    );

    // ── 스냅샷 갱신 + 이벤트 (정수 % 변화시에만 — 이벤트 폭주 차단) ──
    // ★SIM-2 (e): 귀속이 바뀌면 값이 그대로여도 소비자에게 알려야 한다. `changed` 조건에
    // attribution을 넣지 않으면 confident→ambiguous 전이가 HUD에 영영 도달하지 않는다.
    let changed = prev
        .as_ref()
        .map(|p| {
            p.ctx_pct != new.ctx_pct || p.rate != new.rate || p.attribution != new.attribution
        })
        .unwrap_or(true);
    *s.observed_usage.lock().unwrap() = Some(new.clone());
    if changed {
        daemon.bus.publish(
            "usage.updated",
            "usage",
            Some(s.id),
            json!({
                "surface_ref": cys::surface_ref(s.id),
                "role": s.role.lock().unwrap().clone(),
                "agent": new.agent, "ctx_pct": new.ctx_pct, "ctx_tokens": new.ctx_tokens,
                "ctx_window": new.ctx_window, "rate": new.rate, "source": new.source,
                // ★SIM-2 (e): 이 payload는 명시적 `json!`이라 struct 필드 추가만으로는 실리지
                // 않는다 — 여기 키를 더하지 않으면 모호 신호가 소비자에게 **불가시**다.
                "attribution": new.attribution,
            }),
        );
    }
    // 결정론 컨텍스트 임계 — 자기보고(status.set)와 **공유 에지 게이트**(ctx_threshold_armed)
    // 로 발화한다. 분리된 에지 상태를 쓰면 같은 교차에 두 경로가 각각 발화해 master/CSO가
    // cycle-agent를 이중 집행한다. payload source:"observed"로 자기보고 발화와 구분.
    //
    // ★CU-6A 게이트 + C1 고위험 폴백 — 판정은 `observed_fire_source`(순수)가 유일하게 한다.
    if let Some(p) = new.ctx_pct {
        if let Some(src) = observed_fire_source(new.attribution.as_deref(), p) {
            crate::handlers::maybe_fire_context_threshold(
                daemon,
                s,
                p,
                src,
                Some(&new.agent),
                new.attribution.as_deref(),
            );
        }
    }
    if !matches!(new.attribution.as_deref(), None | Some(ATTR_CONFIDENT))
        && prev.as_ref().and_then(|p| p.attribution.as_deref()) != new.attribution.as_deref()
    {
        // **에지 1회만**. 매 틱(2초) 반복 발화는 소음이고, 소음은 곧 무시다.
        daemon.bus.publish(
            "usage.attribution_ambiguous",
            "usage",
            Some(s.id),
            json!({
                "surface_ref": cys::surface_ref(s.id),
                "role": s.role.lock().unwrap().clone(),
                "attribution": new.attribution,
                "session_file": new.session_file,
                "ctx_pct": new.ctx_pct,
                "note": format!(
                    "귀속 모호 — 컨텍스트 임계 게이트 미투입({UNCERTAIN_FIRE_PCT}% 이상은 폴백 발화 · 자기보고 경로는 유지)"
                ),
            }),
        );
    }
}

/// 귀속이 모호·무효여도 관측 발화를 허용하는 **고위험 폴백 임계**.
pub const UNCERTAIN_FIRE_PCT: u8 = 85;

/// 관측 경로의 임계 투입 판정(순수) — `Some(source)`면 그 source로 발화한다.
///
/// ★CU-6A: 귀속이 모호(공유 cwd 폴백 pane 2+)·무효(evicted)인 관측은 원칙적으로 임계에
/// **투입하지 않는다**. 틀린 pane을 순환(/clear)시키면 그 pane의 작업이 날아가기 때문이다.
///
/// ★C1 폴백: 그러나 훅과 statusline이 **동시에** 죽은 환경(윈도우: codex lsof 부재로 귀속이
/// 영구 휴리스틱)에서는 이 게이트가 곧 실명이다 — 100%에 도달해도 아무도 순환시키지 못한다.
/// 그래서 `UNCERTAIN_FIRE_PCT` 이상에서는 모호·무효라도 발화한다. 근거: **오귀속 cycle의
/// 대가(잘못된 pane 순환)보다 100% 방치의 대가(그 pane 작업 전소)가 크며, cycle-agent에는
/// 저장 검증 게이트가 있어 오발화의 실피해가 제한된다**(저장이 안 되면 clear가 실행되지 않는다).
/// source를 `observed-uncertain`으로 나눠 수신측(master/CSO)이 신뢰도를 구분하게 한다.
///
/// ★E2(적대 리뷰 REVISE-2): 폴백 대상은 `ambiguous` **뿐**이다. `evicted` 는 성격이 다르다 —
/// 모호는 "누구 것인지 모른다"(맞을 수도 있다)지만, 축출은 **더 높은 rank 의 다른 pane 이 그
/// 세션을 가져갔다는 확정 사실**이다. 즉 이 관측치는 이 surface 의 것이 아님이 판명된 값이라,
/// 그걸로 임계를 발화하면 **엉뚱한 pane 을 확정적으로 순환**시킨다(폴백의 논거였던 "틀릴 수도
/// 있지만 방치보다 낫다"의 전제 자체가 성립하지 않는다). 게다가 축출된 surface 는 재발견이
/// 트리거되므로(`apply_eviction`), 그 pane 의 진짜 소비는 새 귀속으로 다시 들어온다.
///
/// "max(해당 surface 임계, 85)"의 **surface 임계 쪽 절반은 하류가 이미 강제한다** —
/// `maybe_fire_context_threshold`가 role 오버라이드·env 임계 미만이면 그대로 반환하므로,
/// 여기서 임계를 다시 계산하면 두 곳에서 갈릴 뿐 판정은 같다.
pub fn observed_fire_source(attribution: Option<&str>, pct: u8) -> Option<&'static str> {
    match attribution {
        // `None`(비클로드·구경로)은 판정 자체가 없으므로 종전대로 통과.
        None | Some(ATTR_CONFIDENT) => Some("observed"),
        // 오귀속이 **확정**된 관측 — 어떤 pct 에서도 발화하지 않는다.
        Some(ATTR_EVICTED) => None,
        _ if pct >= UNCERTAIN_FIRE_PCT => Some("observed-uncertain"),
        _ => None,
    }
}

// ───────────────────────── 외부(비-pane) 세션 소비 수집 ─────────────────────────
// cys pane 밖에서 도는 Claude Code 세션(예: 데스크톱 앱·직접 CLI)의 트랜스크립트도
// 비용·효율 집계에 포함한다 — pane 미기동 세션의 모델 사용(fable-5 등)이 CC에서
// 통째로 누락되는 사각지대 해소(2026-07-02 오너 지시).
// 귀속: role = "external"(기본 프로필) / "external:<프로필>"(~/.claude-X → external:X).
// ObservedUsage·ctx 임계 발화는 pane 전용이므로 여기선 소비 적재만 한다.

/// 외부 세션 tail 상태 (수집기 태스크 로컬)
#[derive(Default)]
struct ExternalTails {
    tails: HashMap<PathBuf, TailState>,
    last_sweep: f64,
}

fn external_sweep_secs() -> u64 {
    cys::env_compat("CYS_USAGE_EXTERNAL_SECS")
        .and_then(|v| v.parse().ok())
        .unwrap_or(EXTERNAL_SWEEP_SECS_DEFAULT)
}

fn collect_external(
    daemon: &Arc<Daemon>,
    ext: &mut ExternalTails,
    surfaces: &[Arc<Surface>],
    pane_tails: &HashMap<u64, TailState>,
) {
    let period = external_sweep_secs();
    if period == 0 {
        return; // 명시적 비활성화
    }
    let now = now_epoch();
    if now - ext.last_sweep < period as f64 {
        return;
    }
    ext.last_sweep = now;

    // pane이 소유한 파일 = 등록 transcript + 현재 pane tail 경로 (원경로·정규화 모두 제외)
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut claim = |p: PathBuf| {
        if let Ok(c) = std::fs::canonicalize(&p) {
            claimed.insert(c);
        }
        claimed.insert(p);
    };
    for s in surfaces {
        if let Some(reg) = s.registered_transcript.lock().unwrap().clone() {
            claim(PathBuf::from(reg));
        }
    }
    for t in pane_tails.values() {
        claim(t.path.clone());
    }
    // 미등록 claude pane의 휴리스틱 후보 가드 — (munged cwd, created_at). 이 조합에 걸리는
    // 파일은 pane 수집이 나중에 집어갈 수 있으므로 외부로 세지 않는다(이중계수·오귀속 방지).
    // B-3: pane이 이미 자기 파일을 잡았으면(tail 보유) 가드에서 제외 — 구 구현은 잡은 뒤에도
    // 같은 cwd의 다른 외부 세션들을 영구 배제했다(가드는 "아직 못 잡은" pane만 필요).
    let guards: Vec<(String, f64)> = surfaces
        .iter()
        .filter(|s| !s.exited.load(Ordering::Relaxed))
        .filter(|s| {
            s.agent_meta.lock().unwrap().as_ref().map(|(a, _)| a == "claude").unwrap_or(false)
                && s.registered_transcript.lock().unwrap().is_none()
                && !pane_tails.contains_key(&s.id)
        })
        .map(|s| (claude_project_component(&s.cwd), s.created_at))
        .collect();

    // pane이 소유권을 가져간(또는 삭제된) 파일은 외부 추적에서 해제
    ext.tails.retain(|p, _| !claimed.contains(p) && p.exists());

    // 발견: ~/.claude*/projects/*/*.jsonl 중 최근 활동 파일 (심링크 프로필 중복 제거)
    if let Some(home) = dirs::home_dir() {
        let mut seen_proj: HashSet<PathBuf> = HashSet::new();
        for e in std::fs::read_dir(&home).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name != ".claude" && !name.starts_with(".claude-") {
                continue;
            }
            let projects = e.path().join("projects");
            for proj in std::fs::read_dir(&projects).into_iter().flatten().flatten() {
                let dir = proj.path();
                let canon = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
                if !seen_proj.insert(canon) {
                    continue;
                }
                let comp = proj.file_name().to_string_lossy().into_owned();
                for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let p = f.path();
                    if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                        continue;
                    }
                    if ext.tails.contains_key(&p) || claimed.contains(&p) {
                        continue;
                    }
                    let mt = mtime_epoch(&p);
                    if !external_eligible(now, mt, &comp, &guards) {
                        continue;
                    }
                    // 외부(비-pane) 세션은 surface 귀속 자체가 없다(소비 적재 전용) — 선점
                    // 레지스트리에 들어가지 않으므로 rank는 형식상 결정론 값을 쓴다.
                    ext.tails
                        .insert(p.clone(), TailState::attach(daemon, p, false, RANK_DETERMINISTIC, now));
                }
            }
        }
    }

    // tail + 소비 적재 (pane 경로와 동일 파이프라인 — 락 순서 consumption→analytics)
    for state in ext.tails.values_mut() {
        let lines = read_new_lines(state);
        if lines.is_empty() {
            continue;
        }
        let msgs: Vec<MsgCost> = lines.iter().filter_map(|l| parse_claude_message_cost(l)).collect();
        if msgs.is_empty() {
            continue;
        }
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let sess = state.path.to_string_lossy().into_owned();
        let role = external_role(&state.path);
        let mut c = daemon.consumption.lock().unwrap();
        let alog = daemon.analytics.lock().unwrap();
        for m in msgs {
            let cost = crate::cost::calculate_cost(
                m.input_tokens, m.output, m.cache_creation, m.cache_read, &m.model,
            );
            c.record_message(
                &sess, m.input_tokens + m.cache_creation, m.output, cost, &m.model, now, &today,
            );
            if let Some(conn) = alog.as_ref() {
                crate::analytics::record_usage(
                    conn, &sess, &role, "claude", &m.model, m.input_tokens, m.output,
                    m.cache_creation, m.cache_read, cost, now,
                );
            }
        }
        // 오프셋 영속 — 재시작 시 정확 재개(A-4, pane 경로와 동형)
        if let Some(conn) = alog.as_ref() {
            crate::analytics::save_offset(conn, &sess, state.offset, now);
        }
    }
}

/// 외부 추적 시작 가능 판정 (순수함수 — 테스트 핀): 최근 활동 + pane 휴리스틱 후보 아님
fn external_eligible(now: f64, mtime: f64, comp: &str, guards: &[(String, f64)]) -> bool {
    if now - mtime > EXTERNAL_ACTIVE_SECS {
        return false; // 과거 세션 소급 적재 금지
    }
    // discover_claude_transcript의 후보 조건(mtime + 5.0 >= created_at)과 동일 기준
    !guards.iter().any(|(c, created)| c == comp && mtime + 5.0 >= *created)
}

/// 트랜스크립트 경로의 프로필 → 외부 귀속 role. ~/.claude → "external",
/// ~/.claude-work → "external:work" (by_tier에 그대로 노출)
fn external_role(path: &Path) -> String {
    for comp in path.components() {
        let s = comp.as_os_str().to_string_lossy();
        if let Some(rest) = s.strip_prefix(".claude-") {
            return format!("external:{rest}");
        }
        if s == ".claude" {
            return "external".into();
        }
    }
    "external".into()
}

fn source_label(base: &str, heuristic: bool) -> String {
    if heuristic {
        format!("{base}:heuristic")
    } else {
        base.into()
    }
}

// ───────────────────── CU-6A 귀속 선점 레지스트리(ADR-4) ─────────────────────
//
// 무엇을 고치는가: 같은 프로젝트 디렉터리에서 pane 둘이 돌면 `discover_claude_transcript`는
// **둘 다에게 같은 최신 파일**을 준다(mtime 최신 1개를 고르므로 구조적으로 그렇다). 그 오귀속이
// 조용히 60% 컨텍스트 임계를 발화시키면 master는 엉뚱한 노드에 cycle-agent를 집행한다 —
// 관측이 없는 것보다 나쁜 상태다.
//
// 해법 3층: ①파일 단위 선점 ②소스 신뢰도(rank) 서열 ③그래도 유일하지 않으면 모호로 강등하고
// 게이트에 넣지 않는다. ③이 필요한 이유는 ①만으로는 선점당한 pane이 "다음 후보"로 옮겨가
// **또 다른 남의 파일**을 집기 때문이다(이차 오귀속).

/// statusline(`usage.report`) — 에이전트가 자기 세션을 직접 보고한 **서버 진실**. 최상위.
pub const RANK_STATUSLINE: u8 = 3;
/// 결정론 매핑 — `usage.register`(SessionStart hook이 transcript_path 등록)와 codex lsof fd 직독.
/// "이 pane의 프로세스가 그 파일을 실제로 열고 있다"까지 확인된 매핑이다.
pub const RANK_DETERMINISTIC: u8 = 2;
/// 휴리스틱 폴백 — 디렉터리에서 mtime 최신 파일 집기. **틀릴 수 있는 유일한 등급**이고,
/// 모호 강등·게이트 차단이 겨냥하는 대상이다.
pub const RANK_HEURISTIC: u8 = 1;

/// 유일 매핑으로 확인된 귀속 — 표시·게이트 전부 정상 경로.
pub const ATTR_CONFIDENT: &str = "confident";
/// 같은 프로젝트 디렉터리를 공유하는 폴백 pane이 둘 이상 — 누구 것인지 말할 수 없다.
pub const ATTR_AMBIGUOUS: &str = "ambiguous";
/// 상위 rank가 이 파일을 가져갔다 — 이 귀속은 **틀린 것으로 확정**됐다.
pub const ATTR_EVICTED: &str = "evicted";

/// 선점 시도 결과 — 순수 상태기계의 출력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// 빈 자리·자기 자리 = 성공.
    Granted,
    /// 더 높은(또는 같은) rank가 쥐고 있다 — 이 파일을 관측에 쓰면 안 된다.
    Blocked { holder: u64 },
    /// 탈환 — 저rank 보유자를 축출했다. 호출자는 그 surface의 귀속을 무효화해야 한다.
    Evicted { evicted: u64 },
}

/// 선점 상태기계(순수). 규칙 셋:
/// ①빈 자리·자기 자리 = 성공(rank는 최신 값으로 갱신).
/// ②남이 **더 높거나 같은** rank로 쥐고 있으면 차단. 같은 rank도 차단하는 것이 요점이다 —
///   동급끼리 서로 뺏게 두면 매 틱 소유자가 뒤집히는 플랩이 된다. 선착순 고정이 안정적이다.
/// ③남이 **더 낮은** rank면 탈환. 탈환이 상→하 단방향이므로 되돌아가는 경로가 없고,
///   그래서 플랩이 구조적으로 불가능하다(SIM-2 (b2) 실증).
pub fn claim_session(
    claims: &mut HashMap<PathBuf, (u64, u8)>,
    path: &Path,
    sid: u64,
    rank: u8,
) -> ClaimOutcome {
    match claims.get(path).copied() {
        None => {
            claims.insert(path.to_path_buf(), (sid, rank));
            ClaimOutcome::Granted
        }
        Some((holder, _)) if holder == sid => {
            claims.insert(path.to_path_buf(), (sid, rank));
            ClaimOutcome::Granted
        }
        Some((holder, holder_rank)) if holder_rank >= rank => ClaimOutcome::Blocked { holder },
        Some((holder, _)) => {
            claims.insert(path.to_path_buf(), (sid, rank));
            ClaimOutcome::Evicted { evicted: holder }
        }
    }
}

/// 죽은 surface의 선점을 놓아준다 — 안 하면 종료된 pane이 파일을 영영 쥐고 있어 후속 pane이
/// 자기 세션을 관측하지 못한다(수집 틱 머리에서 1회).
pub fn retain_live_claims(claims: &mut HashMap<PathBuf, (u64, u8)>, live: &HashSet<u64>) {
    claims.retain(|_, (sid, _)| live.contains(sid));
}

/// 후보 목록에서 **상위·동급 rank에 막히지 않은 첫 후보**를 고른다(SIM-2 (a): 폴백은 statusline이
/// 쥔 파일을 못 집는다). 여기서는 **읽기만** 한다 — 실제 선점 전이는 스냅샷 직전 한 곳에서만
/// 수행한다. 두 곳에서 맵을 바꾸면 탈환이 어디서 났는지 추적할 수 없다.
pub fn first_unblocked(
    claims: &HashMap<PathBuf, (u64, u8)>,
    sid: u64,
    candidates: Vec<(PathBuf, u8)>,
) -> Option<(PathBuf, u8)> {
    candidates
        .into_iter()
        .find(|(p, rank)| match claims.get(p) {
            Some((holder, holder_rank)) => *holder == sid || *holder_rank < *rank,
            None => true,
        })
}

/// ★SIM-2 (c) 공유-cwd 모호 강등: **같은 프로젝트 디렉터리에 휴리스틱(rank1) pane이 둘 이상**이면
/// 그 전원을 모호로 내린다. confident는 유일 매핑일 때만이라는 뜻이다.
///
/// 그룹 키가 부모 디렉터리 **이름**인 이유: claude 트랜스크립트는 `~/.claude*/projects/<munged-cwd>/`
/// 에 있고 발견기는 **전 프로필을 가로질러** 최신 파일을 고른다 — 프로필(`.claude` vs `.claude-2`)이
/// 달라도 munged cwd가 같으면 서로의 파일을 집을 수 있으므로, 전체 경로가 아니라 프로젝트
/// 컴포넌트로 묶어야 실제 경합 집합과 일치한다.
pub fn ambiguous_surfaces(claims: &HashMap<PathBuf, (u64, u8)>) -> HashSet<u64> {
    let mut by_project: HashMap<String, HashSet<u64>> = HashMap::new();
    for (path, (sid, rank)) in claims {
        if *rank != RANK_HEURISTIC {
            continue;
        }
        let key = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        by_project.entry(key).or_default().insert(*sid);
    }
    by_project
        .into_values()
        .filter(|g| g.len() >= 2)
        .flatten()
        .collect()
}

/// 탈환 뒷정리 — 축출된 surface의 귀속에 `evicted` 표식을 달고 재발견을 트리거한다.
/// 값을 지우지 않는 이유는 `ObservedUsage::attribution` 주석 참조(조용한 소멸 금지).
fn apply_eviction(
    daemon: &Arc<Daemon>,
    tails: &mut HashMap<u64, TailState>,
    attempts: &mut HashMap<u64, f64>,
    evicted: u64,
    winner: u64,
    path: &Path,
) {
    // surfaces 락을 먼저 놓고 observed_usage를 잡는다 — 핸들러(org.status)의 잠금 순서와 동형.
    let victim = daemon.surfaces.lock().unwrap().get(&evicted).cloned();
    if let Some(v) = victim {
        if let Some(prev) = v.observed_usage.lock().unwrap().as_mut() {
            prev.attribution = Some(ATTR_EVICTED.into());
        }
    }
    // 재발견 트리거: tail을 놓아야 다음 틱에 **다른** 세션 파일을 찾는다. 백오프도 함께 지운다
    // (여긴 지금 막 소유권이 바뀐 순간이라 즉시 재탐색이 옳다).
    tails.remove(&evicted);
    attempts.remove(&evicted);
    daemon.bus.publish(
        "usage.attribution_evicted",
        "usage",
        Some(evicted),
        json!({
            "surface_ref": cys::surface_ref(evicted),
            "session_file": path.to_string_lossy(),
            "taken_by": cys::surface_ref(winner),
            "note": "상위 rank 소스가 이 세션 파일을 가져갔다 — 기존 귀속 무효(재발견 대기)",
        }),
    );
}

// ───────────────────────── 세션 파일 발견 ─────────────────────────

/// 에이전트별 세션 파일 발견 (등록 부재 시) — claude: 프로필 스캔 / codex: lsof → 휴리스틱
fn discover_session_file(s: &Arc<Surface>, agent: &str, bin: &str) -> Option<PathBuf> {
    discover_session_candidates(s, agent, bin)
        .into_iter()
        .next()
        .map(|(p, _)| p)
}

/// ★CU-6A: 발견 결과는 **우선순위 목록**이다(단건이 아니다). 상위 rank가 선점한 후보를 건너뛰고
/// 다음 후보로 갈 수 있어야, 폴백 pane이 남의 파일을 붙잡은 채 굳는 일이 없다.
/// 각 후보에 rank를 함께 붙인다 — lsof 매핑(프로세스가 실제로 연 fd)은 결정론, mtime 스캔은 휴리스틱.
fn discover_session_candidates(s: &Arc<Surface>, agent: &str, bin: &str) -> Vec<(PathBuf, u8)> {
    let bin_base = bin.rsplit(['/', '\\']).next().unwrap_or(bin);
    let (agent_pid, agent_cwd) = find_agent_descendant(s.pid, bin_base);
    let cwd = agent_cwd.unwrap_or_else(|| s.cwd.clone());
    match agent {
        "claude" => discover_claude_transcripts(&cwd, s.created_at)
            .into_iter()
            .map(|p| (p, RANK_HEURISTIC))
            .collect(),
        "codex" => {
            let mut v: Vec<(PathBuf, u8)> = Vec::new();
            if let Some(p) = agent_pid.and_then(discover_codex_rollout_lsof) {
                v.push((p, RANK_DETERMINISTIC));
            }
            for p in discover_codex_rollouts(&cwd, s.created_at) {
                if !v.iter().any(|(q, _)| *q == p) {
                    v.push((p, RANK_HEURISTIC));
                }
            }
            v
        }
        _ => Vec::new(),
    }
}

/// surface 자식 트리에서 에이전트 프로세스의 (pid, cwd)를 찾는다 — 발견 시점에만 호출
/// (전수 프로세스 refresh 비용이 있어 매 틱 호출 금지).
fn find_agent_descendant(surface_pid: u32, bin_base: &str) -> (Option<u32>, Option<String>) {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let pid = crate::governance::collect_descendants(&sys, surface_pid)
        .into_iter()
        .find(|(_, cmdline)| crate::governance::cmdline_matches_agent(cmdline, bin_base))
        .map(|(p, _)| p);
    let cwd = pid.and_then(|p| {
        sys.process(sysinfo::Pid::from_u32(p))
            .and_then(|pr| pr.cwd())
            .map(|c| c.display().to_string())
    });
    (pid, cwd)
}

/// claude 휴리스틱: `~/.claude*` 전 프로필의 projects/<munged>/ 에서 pane 생성 이후 .jsonl 후보를
/// **mtime 내림차순 전부** 돌려준다(심링크 프로필은 canonicalize로 중복 제거).
/// ★CU-6A로 단건→목록이 됐다: 1위가 이미 상위 rank에 선점됐으면 2위를 봐야 하기 때문이다.
/// 목록 자체는 종전과 같은 후보 집합이고 1위도 같으므로, 선점이 없으면 거동은 바이트 동일하다.
fn discover_claude_transcripts(cwd: &str, created_at: f64) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let comp = claude_project_component(cwd);
    let mut found: Vec<(f64, PathBuf)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let Ok(entries) = std::fs::read_dir(&home) else {
        return Vec::new();
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name != ".claude" && !name.starts_with(".claude-") {
            continue;
        }
        let proj = e.path().join("projects").join(&comp);
        let canon = std::fs::canonicalize(&proj).unwrap_or_else(|_| proj.clone());
        if !seen.insert(canon) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&proj) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let mt = mtime_epoch(&p);
            // pane 생성 5초 전까지 허용 (시계 흔들림 여유) — 그 이전 세션은 남의 것
            if mt + 5.0 < created_at {
                continue;
            }
            found.push((mt, p));
        }
    }
    // 최신 우선. 동률은 경로로 안정 정렬 — 순서가 흔들리면 후보 선택이 틱마다 뒤집힌다.
    found.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// codex 결정론: 에이전트 프로세스가 열어둔 rollout 파일 fd를 lsof로 직독 (unix 전용 —
/// 실패·미설치 시 None → 휴리스틱 폴백)
fn discover_codex_rollout_lsof(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .find(|p| p.contains("/sessions/") && p.contains("rollout-") && p.ends_with(".jsonl"))
        .map(PathBuf::from)
}

/// codex 휴리스틱: 최근 3개 날짜 디렉터리에서 session_meta.cwd 일치 + pane 생성 이후 rollout 후보를
/// **mtime 내림차순 전부**(CU-6A — claude 쪽과 같은 이유로 단건→목록).
fn discover_codex_rollouts(cwd: &str, created_at: f64) -> Vec<PathBuf> {
    let Some(base) = dirs::home_dir().map(|h| h.join(".codex").join("sessions")) else {
        return Vec::new();
    };
    let mut day_dirs: Vec<PathBuf> = Vec::new();
    'outer: for y in read_subdirs_desc(&base) {
        for m in read_subdirs_desc(&y) {
            for d in read_subdirs_desc(&m) {
                day_dirs.push(d);
                if day_dirs.len() >= 3 {
                    break 'outer;
                }
            }
        }
    }
    let mut found: Vec<(f64, PathBuf)> = Vec::new();
    for dir in day_dirs {
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let mt = mtime_epoch(&p);
            if mt + 5.0 < created_at {
                continue;
            }
            if rollout_first_line_cwd(&p).as_deref() != Some(cwd) {
                continue;
            }
            found.push((mt, p));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

fn read_subdirs_desc(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

fn rollout_first_line_cwd(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f).read_line(&mut line).ok()?;
    let v: Value = serde_json::from_str(&line).ok()?;
    v["payload"]["cwd"]
        .as_str()
        .or_else(|| v["cwd"].as_str())
        .map(|s| s.to_string())
}

/// (4a) 트랜스크립트 경로에서 agent transcript session_id 추출. claude=파일명 stem, codex=첫줄 payload.id.
/// gemini/agy는 세션파일 포맷 미확인이라 None → boot에서 --continue fallback(회귀 없음).
pub(crate) fn extract_session_id(agent: &str, path: &Path) -> Option<String> {
    match agent {
        "claude" => path.file_stem().and_then(|s| s.to_str()).map(String::from),
        "codex" => {
            let f = std::fs::File::open(path).ok()?;
            let mut line = String::new();
            std::io::BufReader::new(f).read_line(&mut line).ok()?;
            let v: Value = serde_json::from_str(&line).ok()?;
            v["payload"]["id"].as_str().map(String::from)
        }
        _ => None,
    }
}

/// (4a) 세션 발견 + id 추출 묶음 진입점 — discover_session_file로 PathBuf를 얻어 extract_session_id.
/// stash 경로(collect_for)는 이미 발견한 path에 extract_session_id를 직접 적용하므로 현재 미소비.
/// 재발견 없이 id만 필요한 외부 호출(전용 RPC 등) 대비 진입점.
#[allow(dead_code)]
pub(crate) fn discover_session_id(s: &Arc<Surface>, agent: &str, bin: &str) -> Option<String> {
    let path = discover_session_file(s, agent, bin)?;
    extract_session_id(agent, &path)
}

fn mtime_epoch(p: &Path) -> f64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ───────────────────────── 증분 tail ─────────────────────────

/// offset 이후의 완성 라인들을 읽는다. 절단(truncate)·회전 감지 시 마지막 창으로 재정렬,
/// 틱당 읽기 상한 초과 시 따라잡기를 포기하고 점프 (최신 관측만 필요하므로 안전).
fn read_new_lines(state: &mut TailState) -> Vec<String> {
    let Ok(meta) = std::fs::metadata(&state.path) else {
        return Vec::new();
    };
    let len = meta.len();
    if len < state.offset {
        state.offset = len.saturating_sub(FIRST_ATTACH_TAIL);
        state.carry.clear();
    }
    if len == state.offset {
        return Vec::new();
    }
    if len - state.offset > MAX_READ_PER_TICK {
        state.offset = len.saturating_sub(FIRST_ATTACH_TAIL);
        state.carry.clear();
    }
    let to_read = len - state.offset;
    let Ok(mut f) = std::fs::File::open(&state.path) else {
        return Vec::new();
    };
    if f.seek(SeekFrom::Start(state.offset)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::with_capacity(to_read as usize);
    if f.take(to_read).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    state.offset += buf.len() as u64;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let mut combined = std::mem::take(&mut state.carry);
    combined.push_str(&text);
    let ends_nl = combined.ends_with('\n');
    let mut parts: Vec<&str> = combined.split('\n').collect();
    if ends_nl {
        parts.pop(); // 끝 개행 뒤 빈 조각
    } else if let Some(tail) = parts.pop() {
        if tail.len() <= MAX_CARRY {
            state.carry = tail.to_string();
        }
        // 상한 초과 미완성 라인은 폐기 — 다음 개행부터 재동기화
    }
    // RC-10: CRLF 정규화 — Windows 네이티브 프로세스가 쓴 JSONL은 CRLF라 split('\n') 후 각 라인 끝에
    // '\r' 잔류→JSON 파싱 오염. 라인별 trailing '\r' 제거(LF-only는 무영향).
    parts.iter().map(|s| s.trim_end_matches('\r').to_string()).collect()
}

// ───────────────────────── 파서 (순수함수 — 테스트 핀) ─────────────────────────

/// claude 트랜스크립트 assistant 라인 → (현재 컨텍스트 토큰, 모델명).
/// 컨텍스트 = input + cache_read + cache_creation (output 제외 — 공식 문서 공식).
/// isSidechain:true(서브에이전트 트래픽)는 메인 컨텍스트가 아니므로 None.
pub fn parse_claude_line(line: &str) -> Option<(u64, String)> {
    // 빠른 필터: 전체 JSON 파싱 전 후보 라인만 통과 (트랜스크립트 대부분은 비대상)
    if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v["type"].as_str() != Some("assistant") {
        return None;
    }
    if v["isSidechain"].as_bool() == Some(true) {
        return None;
    }
    let u = &v["message"]["usage"];
    if !u.is_object() {
        return None;
    }
    let g = |k: &str| u[k].as_u64().unwrap_or(0);
    let ctx = g("input_tokens") + g("cache_read_input_tokens") + g("cache_creation_input_tokens");
    if ctx == 0 {
        return None; // usage 없는 합성/에러 라인
    }
    let model = v["message"]["model"].as_str().unwrap_or("").to_string();
    Some((ctx, model))
}

/// T7 비용 환산용 — 메시지의 토큰 4종 + 모델. output은 메시지당 가산이라 "오늘 소비"로
/// cost.rs로 USD 환산하고 Consumption 모델믹스에 집계한다.
pub struct MsgCost {
    pub input_tokens: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub model: String,
}

pub fn parse_claude_message_cost(line: &str) -> Option<MsgCost> {
    if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v["type"].as_str() != Some("assistant") || v["isSidechain"].as_bool() == Some(true) {
        return None;
    }
    let u = &v["message"]["usage"];
    if !u.is_object() {
        return None;
    }
    let g = |k: &str| u[k].as_u64().unwrap_or(0);
    let m = MsgCost {
        input_tokens: g("input_tokens"),
        output: g("output_tokens"),
        cache_creation: g("cache_creation_input_tokens"),
        cache_read: g("cache_read_input_tokens"),
        model: v["message"]["model"].as_str().unwrap_or("").to_string(),
    };
    if m.input_tokens == 0 && m.output == 0 && m.cache_creation == 0 && m.cache_read == 0 {
        return None;
    }
    Some(m)
}

/// codex rollout token_count 이벤트 → 턴 소비. last_token_usage가 턴 단위이며
/// input_tokens는 cached 포함이라 (input−cached, cache_read=cached)로 분해한다.
/// model은 이 이벤트에 없어 호출측이 turn_context에서 기억한 값을 채운다(전수조사 A-2).
pub fn parse_codex_message_cost(line: &str) -> Option<MsgCost> {
    if !line.contains("token_count") || !line.contains("last_token_usage") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v["payload"]["type"].as_str() != Some("token_count") {
        return None;
    }
    let u = &v["payload"]["info"]["last_token_usage"];
    if !u.is_object() {
        return None;
    }
    let g = |k: &str| u[k].as_u64().unwrap_or(0);
    let input = g("input_tokens");
    let cached = g("cached_input_tokens").min(input);
    let m = MsgCost {
        input_tokens: input - cached,
        output: g("output_tokens"),
        cache_creation: 0,
        cache_read: cached,
        model: String::new(),
    };
    if m.input_tokens == 0 && m.output == 0 && m.cache_read == 0 {
        return None;
    }
    Some(m)
}

/// codex rollout turn_context 라인의 모델명 (`payload.model` = "gpt-5.5" 등)
pub fn parse_codex_model(line: &str) -> Option<String> {
    if !line.contains("turn_context") || !line.contains("\"model\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v["type"].as_str() != Some("turn_context") {
        return None;
    }
    v["payload"]["model"].as_str().map(|s| s.to_string())
}

/// claude 컨텍스트 윈도우 추정: 기본 200k, 1M 모델([1m])은 1M. CYS_CLAUDE_CTX_WINDOW로
/// 강제 가능 (passive 관측에선 서버 진실값이 없다 — Phase 2 statusline이 정밀값 제공).
pub fn claude_ctx_window(model: &str) -> u64 {
    if let Some(v) = cys::env_compat("CYS_CLAUDE_CTX_WINDOW").and_then(|v| v.parse().ok()) {
        return v;
    }
    if model.contains("[1m]") {
        1_000_000
    } else {
        200_000
    }
}

/// codex token_count 이벤트의 부분 관측 (info / rate_limits가 따로 올 수 있어 Option 병합)
#[derive(Debug, PartialEq)]
pub struct CodexObs {
    pub ctx_tokens: Option<u64>,
    pub ctx_window: Option<u64>,
    pub rate: Option<Vec<RateWindow>>,
}

/// codex rollout 라인 → 컨텍스트·rate limit 관측.
/// 컨텍스트 점유 ≈ last_token_usage.total - reasoning (reasoning 토큰은 컨텍스트에 잔존 안 함).
pub fn parse_codex_line(line: &str) -> Option<CodexObs> {
    if !line.contains("token_count") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let p = &v["payload"];
    if p["type"].as_str() != Some("token_count") {
        return None;
    }
    let info = &p["info"];
    let (ctx_tokens, ctx_window) = if info.is_object() {
        let last = if info["last_token_usage"].is_object() {
            &info["last_token_usage"]
        } else {
            &info["total_token_usage"]
        };
        let total = last["total_tokens"].as_u64().unwrap_or(0);
        let reasoning = last["reasoning_output_tokens"].as_u64().unwrap_or(0);
        (
            Some(total.saturating_sub(reasoning)),
            info["model_context_window"].as_u64(),
        )
    } else {
        (None, None)
    };
    let rl = &p["rate_limits"];
    let rate = if rl.is_object() {
        let mut ws = Vec::new();
        for key in ["primary", "secondary"] {
            let w = &rl[key];
            if let Some(used) = w["used_percent"].as_f64() {
                ws.push(RateWindow {
                    label: window_label(w["window_minutes"].as_u64().unwrap_or(0)),
                    used_pct: used,
                    resets_at: w["resets_at"].as_f64(),
                });
            }
        }
        Some(ws)
    } else {
        None
    };
    if ctx_tokens.is_none() && rate.is_none() {
        return None;
    }
    Some(CodexObs {
        ctx_tokens,
        ctx_window,
        rate,
    })
}

/// rate limit 윈도우 분 → 사람이 읽는 라벨 (300→"5h", 10080→"7d")
pub fn window_label(minutes: u64) -> String {
    match minutes {
        0 => "?".into(),
        m if m % (24 * 60) == 0 => format!("{}d", m / (24 * 60)),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// 사용률 % (반올림·100 상한). window 0은 None — 0 나눗셈·무의미 값 차단.
pub fn pct(tokens: u64, window: u64) -> Option<u8> {
    if window == 0 {
        return None;
    }
    Some(((tokens as f64 / window as f64) * 100.0).round().min(100.0) as u8)
}

/// Claude Code projects/ 디렉터리명 munge — 실측: '/'와 특수문자가 '-'로 치환된다.
/// 단일 소스는 cys 라이브러리(resume 사전검증 게이트와 공유) — 여기선 위임만 한다(로직 중복 금지).
pub fn claude_project_component(cwd: &str) -> String {
    cys::claude_project_component(cwd)
}

// ───────────────────────── T5 Phase 2-B: agy(Antigravity) 쿼터 ─────────────────────────
// agy는 토큰·쿼터를 평문 로컬 파일에 안 남긴다 — 실행 중 프로세스의 로컬 LS RPC(HTTPS,
// self-signed, 127.0.0.1 무인증)로만 노출된다(2026-06-17 라이브 프로브 실측). 포트는 매
// 실행 변동 → lsof로 발견·probe로 검증·캐시. 파일 tail 수집기와 분리된 저빈도 비동기
// 태스크(async curl — tokio 워커 미블로킹). HTTP 클라이언트 의존성을 더하지 않으려 curl
// 셸아웃을 쓴다(codex의 lsof 셸아웃과 동형). 실패·미설치는 graceful(배지 없음 유지).

const AGY_SVC: &str = "exa.language_server_pb.LanguageServerService";

fn agy_poll_secs() -> u64 {
    cys::env_compat("CYS_AGY_POLL_SECS")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(15)
}

/// RetrieveUserQuotaSummary 응답 → RateWindow 벡터 (Gemini 그룹만 — agy 기본 모델).
/// 실측 스키마: `response.groups[].buckets[]{window("5h"|"weekly"), remainingFraction, resetTime}`.
/// used_pct = (1-remainingFraction)*100, weekly→"7d"(claude/codex 배지와 라벨 통일), ISO8601→epoch.
/// PII(GetUserStatus의 name/email)는 건드리지 않는다 — 쿼터 숫자만.
pub fn parse_agy_quota(v: &Value) -> Vec<RateWindow> {
    let mut out = Vec::new();
    let Some(groups) = v["response"]["groups"].as_array() else {
        return out;
    };
    for g in groups {
        if !g["displayName"].as_str().unwrap_or("").contains("Gemini") {
            continue; // 3p(Claude/GPT) 그룹 제외 — agy 기본은 Gemini
        }
        for b in g["buckets"].as_array().into_iter().flatten() {
            let Some(frac) = b["remainingFraction"].as_f64() else {
                continue;
            };
            let label = match b["window"].as_str().unwrap_or("") {
                "5h" => "5h",
                "weekly" => "7d",
                other => other,
            };
            let resets_at = b["resetTime"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp() as f64);
            out.push(RateWindow {
                label: label.to_string(),
                used_pct: ((1.0 - frac) * 100.0).clamp(0.0, 100.0),
                resets_at,
            });
        }
    }
    out.sort_by_key(|r| u8::from(r.label != "5h")); // 5h 먼저, 7d 다음 (배지 순서 안정)
    out
}

/// agy 프로세스가 LISTEN하는 127.0.0.1/localhost 포트 목록 (lsof — codex 패턴 동형, 와일드카드 제외).
async fn agy_listen_ports(pid: u32) -> Vec<u16> {
    let Ok(out) = tokio::process::Command::new("lsof")
        .args(["-nP", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN", "-Fn"])
        .output()
        .await
    else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some(rest) = line.strip_prefix('n') else {
            continue;
        };
        if !(rest.starts_with("localhost:") || rest.starts_with("127.0.0.1:")) {
            continue; // 로컬 바인드만 — agy LS는 localhost
        }
        if let Some(p) = rest.rsplit(':').next().and_then(|s| s.parse::<u16>().ok()) {
            if !ports.contains(&p) {
                ports.push(p);
            }
        }
    }
    ports.truncate(12); // 폭주 가드 — 후보 과다 시 probe 비용 상한
    ports
}

/// 한 포트로 RetrieveUserQuotaSummary 프로브 (async curl -sk, self-signed 수용·2s 타임아웃).
/// 성공 시 Gemini 쿼터 RateWindow, 아니면 None(잘못된 포트·실패).
async fn agy_quota_probe(port: u16) -> Option<Vec<RateWindow>> {
    use crate::state::HideConsole;
    let url = format!("https://127.0.0.1:{port}/{AGY_SVC}/RetrieveUserQuotaSummary");
    let fut = tokio::process::Command::new("curl")
        .args([
            "-sk",
            "--max-time",
            "2",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-H",
            "connect-protocol-version: 1",
            "--data",
            "{}",
            // R-CLI-3(부차): URL이 고정 localhost(포트 숫자)라 실위험은 없으나 동형 패턴 방어심층 —
            // `--` 옵션 종결자로 URL을 위치 인자로 강제한다.
            "--",
            &url,
        ])
        // Windows: 주기 프로브가 콘솔 창을 반복 플래시하지 않게(콘솔 없는 cysd의 콘솔 자식).
        .hide_console()
        .output();
    let out = tokio::time::timeout(Duration::from_secs(3), fut)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let rate = parse_agy_quota(&v);
    if rate.is_empty() {
        None
    } else {
        Some(rate)
    }
}

/// agy 쿼터를 surface.observed_usage(source:"agy-rpc")에 반영 + usage.updated 발행.
/// agy는 context window를 안 주므로 ctx_pct=None(배지는 쿼터만). 임계(context.threshold)는
/// ctx_pct가 없으니 발화 대상 아님.
fn update_agy_usage(daemon: &Arc<Daemon>, s: &Arc<Surface>, rate: Vec<RateWindow>) {
    // CC v2 WS-A: agy 프로브는 항상 신선 생산 — 계정(antigravity/default) 귀속.
    crate::accounts::note_rate(daemon, "gemini", "", &rate, "agy-rpc", now_epoch());
    let new = ObservedUsage {
        agent: "gemini".into(),
        ctx_tokens: None,
        ctx_window: None,
        ctx_pct: None,
        rate,
        source: "agy-rpc".into(),
        session_file: String::new(),
        updated_at: now_epoch(),
        // agy는 세션 파일 매핑 자체가 없다(쿼터 RPC 직독) — 귀속 경합의 대상이 아니므로 미판정.
        // ctx_pct도 None이라 60% 게이트 대상도 아니다.
        attribution: None,
    };
    let changed = s
        .observed_usage
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.rate != new.rate || p.source != new.source)
        .unwrap_or(true);
    *s.observed_usage.lock().unwrap() = Some(new.clone());
    if changed {
        daemon.bus.publish(
            "usage.updated",
            "usage",
            Some(s.id),
            json!({
                "surface_ref": cys::surface_ref(s.id),
                "role": s.role.lock().unwrap().clone(),
                "agent": "gemini", "ctx_pct": Value::Null,
                "rate": new.rate, "source": "agy-rpc",
            }),
        );
    }
}

/// 한 agy surface의 쿼터 수집 — 캐시 포트 우선, 실패 시 lsof 재발견·probe. 전부 실패면 graceful.
async fn collect_agy_for(daemon: &Arc<Daemon>, s: &Arc<Surface>, ports: &mut HashMap<u64, u16>) {
    let mut candidates: Vec<u16> = Vec::new();
    if let Some(p) = ports.get(&s.id) {
        candidates.push(*p);
    }
    let (agy_pid, _) = find_agent_descendant(s.pid, "agy");
    if let Some(pid) = agy_pid {
        for p in agy_listen_ports(pid).await {
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }
    for port in candidates {
        if let Some(rate) = agy_quota_probe(port).await {
            ports.insert(s.id, port);
            update_agy_usage(daemon, s, rate);
            return;
        }
    }
    ports.remove(&s.id); // 캐시 무효화 — 다음 틱에 재발견 (배지는 갱신 안 함 = 정직)
}

/// agy(Antigravity) 쿼터 수집기 — 파일 tail과 분리된 저빈도 비동기 태스크.
pub fn spawn_agy_collector(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut ports: HashMap<u64, u16> = HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(agy_poll_secs())).await;
            let surfaces: Vec<Arc<Surface>> = {
                daemon
                    .surfaces
                    .lock()
                    .unwrap()
                    .values()
                    .filter(|s| !s.exited.load(Ordering::Relaxed))
                    .filter(|s| {
                        s.agent_meta
                            .lock()
                            .unwrap()
                            .as_ref()
                            .map(|(a, _)| a == "gemini")
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            };
            let live: HashSet<u64> = surfaces.iter().map(|s| s.id).collect();
            ports.retain(|sid, _| live.contains(sid));
            for s in surfaces {
                collect_agy_for(&daemon, &s, &mut ports).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 외부(비-pane) 세션 수집 — 귀속·판정 핀 ──

    #[test]
    fn external_role_maps_profile_dirs() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(external_role(&p("/Users/x/.claude/projects/-a/s.jsonl")), "external");
        assert_eq!(
            external_role(&p("/Users/x/.claude-alpha/projects/-a/s.jsonl")),
            "external:alpha"
        );
        assert_eq!(
            external_role(&p("/Users/x/.claude-beta/projects/-a/s.jsonl")),
            "external:beta"
        );
        assert_eq!(external_role(&p("/tmp/other/s.jsonl")), "external");
    }

    #[test]
    fn external_eligible_requires_recent_activity_and_no_pane_candidate() {
        let now = 10_000.0;
        // 최근 활동 아님 → 부적격 (과거 세션 소급 적재 금지)
        assert!(!external_eligible(now, now - EXTERNAL_ACTIVE_SECS - 1.0, "-a", &[]));
        // 최근 활동 + 가드 없음 → 적격
        assert!(external_eligible(now, now - 1.0, "-a", &[]));
        // 같은 comp의 미등록 pane이 있고 mtime이 pane 생성 이후 → pane 휴리스틱 후보라 부적격
        let guards = vec![("-a".to_string(), now - 100.0)];
        assert!(!external_eligible(now, now - 1.0, "-a", &guards));
        // pane 생성 훨씬 이전 mtime(남의 세션 아님이 확실) → 적격
        assert!(external_eligible(now, now - 300.0, "-a", &guards));
        // 다른 comp의 pane은 무관 → 적격
        assert!(external_eligible(now, now - 1.0, "-b", &guards));
    }

    // ── codex 소비 파서: 실측 스키마(2026-07-02 rollout, codex-tui 0.142.5) 핀 ──

    #[test]
    fn codex_token_count_cost_and_model() {
        // input_tokens는 cached 포함 → (input−cached, cache_read=cached)로 분해(A-2)
        let tc = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":19797,"cached_input_tokens":18304,"output_tokens":748,"reasoning_output_tokens":397,"total_tokens":20545},"model_context_window":258400}}}"#;
        let m = parse_codex_message_cost(tc).unwrap();
        assert_eq!(m.input_tokens, 19797 - 18304);
        assert_eq!(m.cache_read, 18304);
        assert_eq!(m.output, 748);
        assert_eq!(m.cache_creation, 0);
        // turn_context에서 모델 캡처 → gpt-5.5 → 정규화 gpt-5-5 → 단가표 적중
        let ctx = r#"{"timestamp":"t","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/x"}}"#;
        assert_eq!(parse_codex_model(ctx).unwrap(), "gpt-5.5");
        assert!(crate::cost::has_pricing("gpt-5.5"), "gpt-5.5 단가표 적중 필요");
        // 비대상 라인은 None
        assert!(parse_codex_message_cost(r#"{"type":"event_msg","payload":{"type":"agent_message"}}"#).is_none());
        assert!(parse_codex_model(r#"{"type":"session_meta","payload":{}}"#).is_none());
    }

    // ── claude 파서: 실측 스키마(2026-06-13, CLI 2.1.176) 핀 ──

    fn claude_line(extra: &str, usage: &str) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":false,"requestId":"req_1","sessionId":"s","timestamp":"t"{extra},"message":{{"model":"claude-fable-5","usage":{usage}}}}}"#
        )
    }

    #[test]
    fn claude_ctx_is_input_plus_both_caches_excluding_output() {
        // 공식 statusline 문서 공식: used = input + cache_creation + cache_read (output 제외).
        // 실측값 2+82077+717=82796 — output_tokens가 합산되면 이 핀이 깨진다.
        let line = claude_line(
            "",
            r#"{"input_tokens":2,"cache_creation_input_tokens":717,"cache_read_input_tokens":82077,"output_tokens":999}"#,
        );
        let (ctx, model) = parse_claude_line(&line).expect("assistant usage 라인 파싱 실패");
        assert_eq!(ctx, 82_796);
        assert_eq!(model, "claude-fable-5");
    }

    #[test]
    fn claude_sidechain_lines_are_excluded() {
        // 서브에이전트(isSidechain:true) 트래픽은 메인 컨텍스트가 아니다 — 섞이면
        // 메인 pane 배지가 서브에이전트 컨텍스트로 오염된다.
        let line = claude_line("", r#"{"input_tokens":50000}"#).replace(
            r#""isSidechain":false"#,
            r#""isSidechain":true"#,
        );
        assert_eq!(parse_claude_line(&line), None);
    }

    #[test]
    fn claude_non_assistant_and_zero_usage_skipped() {
        assert_eq!(
            parse_claude_line(r#"{"type":"user","message":{"usage":{"input_tokens":5}}}"#),
            None,
            "user 라인은 무시"
        );
        let zero = claude_line("", r#"{"input_tokens":0,"output_tokens":3}"#);
        assert_eq!(parse_claude_line(&zero), None, "입력측 0은 합성 라인 — 무시");
        assert_eq!(parse_claude_line("not json"), None);
        assert_eq!(parse_claude_line(""), None);
    }

    #[test]
    fn claude_window_default_and_1m_variant() {
        // ★테스트 격리: 런타임 환경(예: Claude Code 세션)이 CYS_CLAUDE_CTX_WINDOW(또는
        // JAVIS_/AITERM_ 호환 별칭)을 설정하면 env 오버라이드가 모델 기본값을 덮어 이 핀이
        // 거짓 실패한다. 모델 기반 분기만 검증하도록 해당 env를 제거 후 단언하고 복원한다.
        let keys = [
            "CYS_CLAUDE_CTX_WINDOW",
            "JAVIS_CLAUDE_CTX_WINDOW",
            "AITERM_CLAUDE_CTX_WINDOW",
        ];
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in keys {
            std::env::remove_var(k);
        }
        assert_eq!(claude_ctx_window("claude-fable-5"), 200_000);
        assert_eq!(claude_ctx_window("claude-sonnet-4-6[1m]"), 1_000_000);
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }

    // ── codex 파서: 실측 스키마(2026-06-13, codex-cli 0.139.0) 핀 ──

    const CODEX_FULL: &str = r#"{"timestamp":"2026-06-12T23:38:22.044Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":26788,"cached_input_tokens":2432,"output_tokens":508,"reasoning_output_tokens":352,"total_tokens":27296},"last_token_usage":{"input_tokens":26788,"cached_input_tokens":2432,"output_tokens":508,"reasoning_output_tokens":352,"total_tokens":27296},"model_context_window":258400},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":13.0,"window_minutes":300,"resets_at":1781314865},"secondary":{"used_percent":3.0,"window_minutes":10080,"resets_at":1781781650},"credits":null,"individual_limit":null,"plan_type":"plus","rate_limit_reached_type":null}}}"#;

    #[test]
    fn codex_full_event_yields_ctx_and_both_rate_windows() {
        let obs = parse_codex_line(CODEX_FULL).expect("token_count 파싱 실패");
        // 컨텍스트 = total - reasoning (27296 - 352)
        assert_eq!(obs.ctx_tokens, Some(26_944));
        assert_eq!(obs.ctx_window, Some(258_400));
        let rate = obs.rate.expect("rate_limits 누락");
        assert_eq!(rate.len(), 2);
        assert_eq!(rate[0].label, "5h");
        assert_eq!(rate[0].used_pct, 13.0);
        assert_eq!(rate[0].resets_at, Some(1_781_314_865.0));
        assert_eq!(rate[1].label, "7d");
        assert_eq!(rate[1].used_pct, 3.0);
    }

    #[test]
    fn codex_rate_only_event_keeps_ctx_none() {
        // 일부 모드는 info 없이 rate_limits만 싣는다 (codex #14880) — 부분 관측 허용
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":50.5,"window_minutes":300,"resets_at":1781314865}}}}"#;
        let obs = parse_codex_line(line).expect("rate-only 파싱 실패");
        assert_eq!(obs.ctx_tokens, None);
        assert_eq!(obs.rate.as_ref().map(|r| r.len()), Some(1));
        assert_eq!(obs.rate.unwrap()[0].used_pct, 50.5);
    }

    #[test]
    fn codex_non_token_count_lines_skipped() {
        assert_eq!(
            parse_codex_line(r#"{"type":"session_meta","payload":{"cwd":"/x"}}"#),
            None
        );
        assert_eq!(
            parse_codex_line(r#"{"type":"event_msg","payload":{"type":"agent_message"}}"#),
            None
        );
        // payload.type은 token_count지만 내용이 전무 — None
        assert_eq!(
            parse_codex_line(
                r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#
            ),
            None
        );
    }

    #[test]
    fn window_labels_match_known_codex_windows() {
        assert_eq!(window_label(300), "5h");
        assert_eq!(window_label(10080), "7d");
        assert_eq!(window_label(90), "90m");
        assert_eq!(window_label(0), "?");
        assert_eq!(window_label(1440), "1d");
    }

    #[test]
    fn pct_rounds_and_caps() {
        assert_eq!(pct(82_796, 200_000), Some(41));
        assert_eq!(pct(0, 200_000), Some(0));
        assert_eq!(pct(300_000, 200_000), Some(100), "윈도우 초과는 100 상한");
        assert_eq!(pct(1, 0), None, "윈도우 0 — 0 나눗셈 차단");
    }

    #[test]
    fn munge_matches_observed_directory_names() {
        // 실측: /Users/user/Desktop/CYSjavis/cys-terminal → -Users-user-Desktop-CYSjavis-cys-terminal
        assert_eq!(
            claude_project_component("/Users/user/Desktop/CYSjavis/cys-terminal"),
            "-Users-user-Desktop-CYSjavis-cys-terminal"
        );
        // 비ASCII·특수문자는 각각 '-' (보수 구현 — 휴리스틱 폴백 전용)
        assert_eq!(claude_project_component("/tmp/a.b_c"), "-tmp-a-b-c");
    }

    // ── 증분 tail: 회전·부분라인·따라잡기 한도 ──

    #[test]
    fn read_new_lines_handles_partial_lines_and_truncation() {
        let dir = std::env::temp_dir().join(format!("cys-usage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(&path, "line1\nline2\npart").unwrap();
        let mut st = TailState {
            path: path.clone(),
            offset: 0,
            carry: String::new(),
            heuristic: false,
            last_discovery: 0.0,
            server_ctx_window: None,
            codex_model: None,
            rank: RANK_DETERMINISTIC,
        };
        let lines = read_new_lines(&mut st);
        assert_eq!(lines, vec!["line1".to_string(), "line2".to_string()]);
        assert_eq!(st.carry, "part", "미완성 라인은 carry로 보류");
        // 이어서 완성 — carry와 합쳐 한 줄로
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, b"ial\n").unwrap();
        drop(f);
        assert_eq!(read_new_lines(&mut st), vec!["partial".to_string()]);
        // 절단(truncate) — offset 재정렬 후 새 내용 읽힘
        std::fs::write(&path, "fresh\n").unwrap();
        assert_eq!(read_new_lines(&mut st), vec!["fresh".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollout_first_line_cwd_reads_session_meta() {
        let dir = std::env::temp_dir().join(format!("cys-usage-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-x.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"t","type":"session_meta","payload":{"id":"u","cwd":"/work/dir","cli_version":"0.139.0"}}
{"type":"event_msg","payload":{"type":"token_count"}}
"#,
        )
        .unwrap();
        assert_eq!(rollout_first_line_cwd(&path).as_deref(), Some("/work/dir"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T5 Phase 2-B: agy RetrieveUserQuotaSummary 파싱 핀 — 2026-06-17 라이브 실측 스키마.
    /// Gemini 그룹만 추출(3p Claude/GPT 제외)·weekly→"7d"·used_pct=(1-remainingFraction)*100·
    /// resetTime ISO8601→epoch. PII(GetUserStatus의 name/email)는 만지지 않는다.
    #[test]
    fn agy_quota_parses_gemini_group_only() {
        let v: Value = serde_json::from_str(
            r#"{"response":{"groups":[
            {"displayName":"Gemini Models","buckets":[
                {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.9484245,"resetTime":"2026-06-19T20:29:38Z"},
                {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.993488,"resetTime":"2026-06-16T21:04:55Z"}]},
            {"displayName":"Claude and GPT models","buckets":[
                {"bucketId":"3p-5h","window":"5h","remainingFraction":1.0,"resetTime":"2026-06-16T21:25:07Z"}]}]}}"#,
        )
        .unwrap();
        let r = parse_agy_quota(&v);
        assert_eq!(r.len(), 2, "Gemini 그룹 2버킷만 — 3p 그룹 제외");
        assert_eq!(r[0].label, "5h", "5h 먼저 정렬");
        assert!((r[0].used_pct - 0.6512).abs() < 0.01, "5h used≈0.65: {}", r[0].used_pct);
        assert_eq!(r[1].label, "7d", "weekly→7d 라벨 통일");
        assert!((r[1].used_pct - 5.1576).abs() < 0.01, "weekly used≈5.16: {}", r[1].used_pct);
        assert!(r[0].resets_at.is_some(), "resetTime ISO8601→epoch 변환");
    }

    #[test]
    fn agy_quota_empty_on_no_groups_or_3p_only() {
        assert!(parse_agy_quota(&json!({})).is_empty());
        assert!(parse_agy_quota(&json!({"response":{"groups":[]}})).is_empty());
        // 3p 그룹만 있으면 빈 벡터 (Gemini 그룹 없음)
        let only3p = json!({"response":{"groups":[
            {"displayName":"Claude and GPT models","buckets":[
                {"bucketId":"3p-5h","window":"5h","remainingFraction":1.0}]}]}});
        assert!(parse_agy_quota(&only3p).is_empty());
    }

    /// T7: 메시지별 토큰 4종 + 모델 파싱(cost 환산 입력) — cache_read·model 포함, sidechain·전부0은 None.
    #[test]
    fn claude_message_cost_parse() {
        let line = r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-4-8","usage":{"input_tokens":1000,"cache_creation_input_tokens":2000,"cache_read_input_tokens":50000,"output_tokens":300}}}"#;
        let m = parse_claude_message_cost(line).unwrap();
        assert_eq!((m.input_tokens, m.cache_creation, m.cache_read, m.output), (1000, 2000, 50000, 300));
        assert_eq!(m.model, "claude-opus-4-8");
        let sc = line.replace("\"isSidechain\":false", "\"isSidechain\":true");
        assert!(parse_claude_message_cost(&sc).is_none(), "sidechain 제외");
        assert!(
            parse_claude_message_cost(r#"{"type":"assistant","message":{"usage":{"input_tokens":0,"output_tokens":0}}}"#).is_none(),
            "전부 0은 None"
        );
    }

    /// T6: 소비 트래커 — 오늘 누적·세션 집계·최근창·스파크라인·날짜변경 리셋.
    #[test]
    fn consumption_today_recent_sparkline_reset() {
        use crate::state::Consumption;
        let mut c = Consumption::default();
        let now = 1_000_000.0;
        c.record_message("/s/a.jsonl", 100, 50, 0.5, "claude-opus-4-8", now - 7200.0, "2026-06-17");
        c.record_message("/s/a.jsonl", 200, 100, 1.0, "claude-opus-4-8", now - 1800.0, "2026-06-17");
        c.record_message("/s/b.jsonl", 10, 5, 0.1, "claude-haiku-4-5", now, "2026-06-17");
        assert_eq!(c.today_msgs, 3);
        assert_eq!(c.today_tokens, 100 + 50 + 200 + 100 + 10 + 5);
        assert_eq!(c.today_input, 100 + 200 + 10);
        assert!((c.today_cost_usd - 1.6).abs() < 1e-9, "비용 합산 0.5+1.0+0.1");
        assert_eq!(c.model_tokens.get("claude-opus-4-8").copied(), Some(450), "opus 토큰 150+300");
        assert_eq!(c.model_tokens.get("claude-haiku-4-5").copied(), Some(15));
        assert_eq!(c.sessions.len(), 2, "세션 a,b 2개");
        assert_eq!(c.recent_tokens(now, 3600.0), 300 + 15, "최근 1h = 30m전(300)+now(15)");
        assert_eq!(c.sparkline(now, 12, 43200.0).iter().sum::<u64>(), 150 + 300 + 15, "12h 전부 포함");
        c.record_message("/s/c.jsonl", 1, 1, 0.2, "claude-opus-4-8", now + 100.0, "2026-06-18");
        assert_eq!(c.today_msgs, 1, "날짜 변경 시 오늘 카운터 리셋");
        assert_eq!(c.sessions.len(), 1, "세션도 리셋");
        assert!((c.today_cost_usd - 0.2).abs() < 1e-9, "비용도 리셋");
        assert_eq!(c.model_tokens.len(), 1, "모델믹스도 리셋");
    }

    // ─────────────── CU-6A 귀속 선점 상태기계 (SIM-2 (a)(b)(b2)(c)(d)) ───────────────
    //
    // 이 다섯은 "같은 프로젝트 디렉터리에서 pane 둘이 돌 때 남의 컨텍스트를 내 것으로 그리고
    // 엉뚱한 노드를 순환시킨다"는 실사고 경로를 각각 한 마디씩 끊는다. 데몬·파일시스템 없이
    // 순수 상태기계로 재현하는 이유는, 이 경합이 실기에서는 재현 자체가 어렵기 때문이다.

    fn claims_of(items: &[(&str, u64, u8)]) -> HashMap<PathBuf, (u64, u8)> {
        items.iter().map(|(p, sid, r)| (PathBuf::from(*p), (*sid, *r))).collect()
    }

    /// (a) statusline(rank3)이 쥔 파일을 폴백(rank1)이 **못 집는다**.
    /// 이게 뚫리면 statusline 보고를 가진 정상 pane의 세션이 옆 pane에게 도둑맞는다.
    #[test]
    fn sim2a_fallback_cannot_take_a_statusline_claimed_file() {
        let f = "/h/.claude/projects/proj/a.jsonl";
        let mut claims = claims_of(&[(f, 7, RANK_STATUSLINE)]);
        assert_eq!(
            claim_session(&mut claims, Path::new(f), 9, RANK_HEURISTIC),
            ClaimOutcome::Blocked { holder: 7 }
        );
        assert_eq!(claims[&PathBuf::from(f)], (7, RANK_STATUSLINE), "차단인데 소유자가 바뀌었다");
        // 후보 선택 단계에서도 같은 판정이어야 한다 — 애초에 고르지 않는 것이 1차 방어다.
        let picked = first_unblocked(
            &claims,
            9,
            vec![(PathBuf::from(f), RANK_HEURISTIC), (PathBuf::from("/h/.claude/projects/proj/b.jsonl"), RANK_HEURISTIC)],
        );
        assert_eq!(picked.map(|(p, _)| p), Some(PathBuf::from("/h/.claude/projects/proj/b.jsonl")),
                   "선점된 1위를 건너뛰고 2위를 골라야 한다");
    }

    /// (b) 폴백이 먼저 쥐고 있어도 statusline이 **후착 탈환**한다(무효화는 그 순간 1회).
    /// 서열이 시간순이면 먼저 틀린 매핑이 영원히 이긴다 — 그래서 rank가 시간보다 세다.
    #[test]
    fn sim2b_higher_rank_evicts_a_prior_fallback_holder() {
        let f = "/h/.claude/projects/proj/a.jsonl";
        let mut claims = claims_of(&[(f, 9, RANK_HEURISTIC)]);
        assert_eq!(
            claim_session(&mut claims, Path::new(f), 7, RANK_STATUSLINE),
            ClaimOutcome::Evicted { evicted: 9 }
        );
        assert_eq!(claims[&PathBuf::from(f)], (7, RANK_STATUSLINE));
    }

    /// (b2) 탈환 후 폴백이 다시 청구해도 막힌다 = **플랩 구조적 0**.
    /// 탈환이 상→하 단방향이라 되돌아가는 경로 자체가 없다(동급도 선착순 고정).
    #[test]
    fn sim2b2_no_flap_after_eviction() {
        let f = "/h/.claude/projects/proj/a.jsonl";
        let mut claims = claims_of(&[(f, 7, RANK_STATUSLINE)]);
        for _ in 0..5 {
            assert_eq!(
                claim_session(&mut claims, Path::new(f), 9, RANK_HEURISTIC),
                ClaimOutcome::Blocked { holder: 7 },
                "재청구가 통과하면 매 틱 소유자가 뒤집힌다"
            );
        }
        // 동급끼리도 선착순 고정 — 서로 뺏으면 그 자체가 플랩이다.
        let mut same = claims_of(&[(f, 9, RANK_HEURISTIC)]);
        assert_eq!(
            claim_session(&mut same, Path::new(f), 11, RANK_HEURISTIC),
            ClaimOutcome::Blocked { holder: 9 }
        );
        // 자기 재청구는 언제나 통과(갱신) — 이게 막히면 정상 pane이 매 틱 관측을 잃는다.
        assert_eq!(
            claim_session(&mut same, Path::new(f), 9, RANK_HEURISTIC),
            ClaimOutcome::Granted
        );
    }

    /// (c) 같은 프로젝트 디렉터리에 폴백 pane이 둘 이상 = **전원 모호**.
    /// 선점만으로는 부족하다 — 밀려난 pane은 다음 후보(= 또 다른 남의 파일)를 집기 때문이다.
    /// 프로필이 달라도(`.claude` vs `.claude-2`) munged cwd가 같으면 한 경합 집합이다.
    #[test]
    fn sim2c_shared_project_dir_downgrades_all_fallbacks_to_ambiguous() {
        let claims = claims_of(&[
            ("/h/.claude/projects/-w-proj/a.jsonl", 7, RANK_HEURISTIC),
            ("/h/.claude-2/projects/-w-proj/b.jsonl", 9, RANK_HEURISTIC),
            // 다른 프로젝트의 단독 폴백 — 유일 매핑이므로 confident로 남아야 한다.
            ("/h/.claude/projects/-w-other/c.jsonl", 11, RANK_HEURISTIC),
            // 같은 디렉터리라도 결정론(rank2) 매핑은 경합 대상이 아니다.
            ("/h/.claude/projects/-w-proj/d.jsonl", 13, RANK_DETERMINISTIC),
        ]);
        let amb = ambiguous_surfaces(&claims);
        assert!(amb.contains(&7) && amb.contains(&9), "공유 디렉터리 폴백 전원이 모호여야 한다: {amb:?}");
        assert!(!amb.contains(&11), "단독 폴백까지 모호로 내리면 정상 관측이 통째로 죽는다");
        assert!(!amb.contains(&13), "결정론 매핑은 모호 강등 대상이 아니다");
        // 단독 폴백 하나뿐이면 모호는 없다(회귀: 기본 상태에서 게이트가 닫히면 안 된다).
        let solo = claims_of(&[("/h/.claude/projects/-w-proj/a.jsonl", 7, RANK_HEURISTIC)]);
        assert!(ambiguous_surfaces(&solo).is_empty());
    }

    /// (d) 게이트 판정 — **폴백 임계 미만 구간**: 모호·무효 71%는 임계에 미투입, confident 61%는
    /// 발화, `None`(구경로)은 통과. 이 표가 CU-6A의 존재 이유다 — 틀린 pane을 /clear 시키면
    /// 그 노드의 작업이 날아간다. 85% 이상 구간은 C1 폴백이 담당한다(아래 별도 테스트).
    #[test]
    fn sim2d_threshold_gate_admits_only_confident_or_unjudged() {
        // collect_for가 **실제로 부르는 함수**를 단정한다 — 식을 복제하면 언젠가 갈린다.
        assert_eq!(observed_fire_source(Some(ATTR_CONFIDENT), 61), Some("observed"));
        assert_eq!(observed_fire_source(None, 61), Some("observed"), "비클로드·구경로(미판정)는 종전대로 통과 — 회귀 0");
        assert_eq!(observed_fire_source(Some(ATTR_AMBIGUOUS), 71), None, "모호 71%가 임계를 발화하면 엉뚱한 노드가 순환된다");
        assert_eq!(observed_fire_source(Some(ATTR_EVICTED), 71), None, "무효화된 귀속은 발화 금지(E2: 폴백 구간 포함 전면)");
        // 폴백 경계 바로 아래는 여전히 차단 — off-by-one이 게이트를 통째로 여는 것을 막는다.
        assert_eq!(observed_fire_source(Some(ATTR_AMBIGUOUS), UNCERTAIN_FIRE_PCT - 1), None);
    }

    /// ★C1 고위험 폴백: 훅+statusline이 동시에 죽은 환경(윈도우: codex lsof 부재 → 영구 휴리스틱)
    /// 에서 CU-6A 게이트는 곧 실명이다 — 100%에 도달해도 아무도 순환시키지 못하고 그 pane의
    /// 작업이 통째로 날아간다. 85% 이상은 모호·무효라도 발화하되 source를 나눠 신뢰도를 알린다.
    #[test]
    fn c1_uncertain_attribution_still_fires_at_high_pct() {
        assert_eq!(
            observed_fire_source(Some(ATTR_AMBIGUOUS), UNCERTAIN_FIRE_PCT),
            Some("observed-uncertain"),
            "ambiguous {UNCERTAIN_FIRE_PCT}%가 막히면 그 pane은 100%까지 방치된다"
        );
        assert_eq!(
            observed_fire_source(Some(ATTR_AMBIGUOUS), 100),
            Some("observed-uncertain")
        );
        // 확실한 귀속은 폴백 구간에서도 source가 격하되지 않는다(수신측 신뢰도 판단 재료).
        assert_eq!(observed_fire_source(Some(ATTR_CONFIDENT), 100), Some("observed"));
    }

    /// ★E2 회귀 핀: `evicted` 는 **폴백 구간에서도** 발화하지 않는다.
    /// ambiguous("누구 것인지 모른다")와 달리 evicted 는 "이 surface 것이 아님이 확정됐다"는
    /// 사실이라, 발화하면 엉뚱한 pane 을 확정적으로 /clear 시킨다 — 폴백의 논거("틀릴 수도
    /// 있지만 방치보다 낫다")가 성립하지 않는 유일한 값이다.
    #[test]
    fn c1_fallback_excludes_evicted_attribution() {
        for pct in [0u8, 60, UNCERTAIN_FIRE_PCT - 1, UNCERTAIN_FIRE_PCT, 99, 100] {
            assert_eq!(
                observed_fire_source(Some(ATTR_EVICTED), pct),
                None,
                "evicted {pct}% 발화 = 오귀속 확정 pane 순환(작업 전소)"
            );
        }
        // 대조군: 같은 pct 에서 ambiguous 는 폴백을 그대로 받는다(폴백 자체는 살아 있다).
        assert_eq!(
            observed_fire_source(Some(ATTR_AMBIGUOUS), 100),
            Some("observed-uncertain")
        );
    }

    /// 폴백은 **에지 게이트를 우회하지 않는다** — 판정 함수는 순수하고, 발화는 여전히 공유
    /// `ctx_threshold_armed` 게이트 한 곳(`maybe_fire_context_threshold`)만 지난다. 폴백을
    /// 급히 넣다가 게이트를 우회하는 인라인 발화를 심으면 같은 교차에 cycle-agent가 이중
    /// 집행된다(에지 1회성의 실제 동작 핀은 handlers.rs의 `c1_*_fires_once_per_edge`).
    #[test]
    fn c1_fallback_does_not_bypass_the_shared_edge_gate() {
        let src = include_str!("usage.rs");
        // 테스트 모듈은 판정 대상이 아니다 — 자기 자신의 문자열 리터럴까지 세면 안 된다.
        let prod = src.split("#[cfg(test)]").next().expect("소스 본문");
        let head = prod
            .split("pub fn observed_fire_source")
            .nth(1)
            .expect("observed_fire_source 함수가 사라졌다");
        let body = &head[..head.find("\n}").expect("함수 본문 끝을 찾지 못했다")];
        assert!(
            !body.contains("ctx_threshold_armed") && !body.contains("publish"),
            "판정 함수가 에지 상태·버스를 직접 만지면 이중 집행 차단이 무너진다: {body}"
        );
        // 관측 경로의 발화 진입점은 하나뿐이어야 한다(경로 복제 = 이중 발화).
        assert_eq!(
            prod.matches("crate::handlers::maybe_fire_context_threshold").count(),
            1
        );
    }

    /// 죽은 pane의 선점 해제 — 안 하면 종료된 pane이 파일을 영영 쥐고 있어 후속 pane이 자기
    /// 세션을 관측하지 못한다(부팅 후 첫 pane이 조용히 실명하는 형태로 나타난다).
    #[test]
    fn dead_surfaces_release_their_claims() {
        let mut claims = claims_of(&[
            ("/h/.claude/projects/p/a.jsonl", 7, RANK_HEURISTIC),
            ("/h/.claude/projects/p/b.jsonl", 9, RANK_STATUSLINE),
        ]);
        let live: HashSet<u64> = [9u64].into_iter().collect();
        retain_live_claims(&mut claims, &live);
        assert_eq!(claims.len(), 1);
        assert!(claims.contains_key(&PathBuf::from("/h/.claude/projects/p/b.jsonl")));
    }

    /// ★SIM-2 (e): 이벤트 페이로드는 명시적 `json!`이라 struct 필드 추가만으로는 실리지 않는다.
    /// 소비자(HUD)가 모호 신호를 볼 수 있는 유일한 통로이므로 키 존재를 박제한다.
    #[test]
    fn usage_updated_payload_carries_attribution_key() {
        let src = include_str!("usage.rs");
        let publish = src
            .split("\"usage.updated\",")
            .nth(1)
            .expect("usage.updated publish 지점이 사라졌다");
        // json! 블록 본문만 잘라 본다(바이트 슬라이스는 한글 주석에서 문자 경계를 깬다).
        let payload = publish.split("}),").next().expect("json! 블록");
        assert!(
            payload.contains("\"attribution\""),
            "usage.updated 페이로드에 attribution 키가 없다 — 모호 신호가 소비자에게 불가시가 된다:\n{payload}"
        );
    }
}
