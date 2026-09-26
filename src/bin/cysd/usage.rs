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

/// 창 라벨(`5h`·`7d`·`300m` — [`window_label`] 규칙) → 창 길이(초). 모르는 라벨(`?` 등)은 None.
/// 라벨은 좌석 보고가 실어 오는 임의 문자열일 수 있다 — 바이트 절단 대신 **마지막 글자**로 가른다(다바이트 라벨에서
/// 문자 경계 panic 금지).
pub fn window_secs(label: &str) -> Option<f64> {
    let unit = label.chars().last()?;
    let num = &label[..label.len() - unit.len_utf8()];
    let n: f64 = num.parse::<u32>().ok().filter(|n| *n > 0)?.into();
    match unit {
        'd' => Some(n * 86400.0),
        'h' => Some(n * 3600.0),
        'm' => Some(n * 60.0),
        _ => None,
    }
}

/// 리셋 시각이 epoch 초로 보이는 하한(2001-09). 이보다 작은 값은 단위가 다른 원천(상대 초 등)이라 판단 근거로 쓰지 않는다.
const RESET_EPOCH_FLOOR: f64 = 1.0e9;

/// ★fatal-fix R3-2 · ROLE-3: 이 창 관측이 **아직 경보 근거인가**(순수 — 핀). 리셋 시각이 지난 창은 아니다 — UI 의
/// '리셋됨' 규칙과 같다(종전에는 유휴 agy 좌석의 '100%'가 리셋 뒤에도 남아 30분마다 crit 로 다시 울렸다 · agy 상태줄은
/// 상태가 바뀔 때만 불려 스스로 지워 주지 않는다). 리셋 시각이 없거나 epoch 초로 보이지 않으면, 관측 나이가 창 길이를
/// 넘을 때만 뺀다(그 창은 리셋됐을 수밖에 없다) · 창 길이도 모르면 남긴다(지우는 쪽으로 오판하지 않는다).
/// `observed_at` = 그 값을 관측한 시각(0 = 모름 → 나이 판정 없음).
pub fn rate_window_live(w: &RateWindow, observed_at: f64, now: f64) -> bool {
    match w.resets_at {
        Some(r) if r.is_finite() && r >= RESET_EPOCH_FLOOR => r > now,
        _ => match window_secs(&w.label) {
            Some(len) if observed_at > 0.0 => now - observed_at <= len,
            _ => true,
        },
    }
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
}

impl TailState {
    /// 새 tail — 영속 오프셋(analytics tail_offsets)이 있으면 거기서 정확 재개해
    /// 재시작 시 마지막 256KB 재파싱→DB 중복 INSERT(전수조사 A-4)를 근절한다.
    fn attach(daemon: &Arc<Daemon>, path: PathBuf, heuristic: bool, now: f64) -> Self {
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
        TailState { path, offset, carry: String::new(), heuristic, last_discovery: now, server_ctx_window: None, codex_model: None }
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
    let desired: Option<(PathBuf, bool)> = if let Some(reg) = registered {
        Some((PathBuf::from(reg), false))
    } else {
        let need_discovery = match tails.get(&s.id) {
            None => true,
            Some(t) => needs_rediscovery(t.path.exists(), t.heuristic, now, t.last_discovery),
        };
        let existing = || {
            tails
                .get(&s.id)
                .filter(|t| t.path.exists())
                .map(|t| (t.path.clone(), t.heuristic))
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
                discover_session_file(s, agent, bin)
                    .map(|p| (p, true))
                    .or_else(existing)
            }
        } else {
            existing()
        }
    };
    let Some((path, heuristic)) = desired else {
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
        tails.insert(s.id, TailState::attach(daemon, path.clone(), heuristic, now));
        // 새 세션 파일 = 새 세션 — 에지 게이트 재무장. 직전 세션이 임계 위에서 끝났어도
        // 새 세션이 곧장 임계 이상으로 시작하면(거대 지침 재주입) 발화해야 한다.
        s.ctx_threshold_armed.store(true, Ordering::Relaxed);
    } else if let Some(t) = tails.get_mut(&s.id) {
        t.heuristic = heuristic;
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
        // ★R1-blocking-1: 신규 줄이 없어도 **낡은 매핑은 값을 비운다**. 세션 교체 후 옛 파일에는
        //   줄이 붙지 않으므로 여기서 그냥 반환하면 이전 numeric snapshot 이 무기한 남는다
        //   (B6 목적 미달 — codex 감사). statusline 이 신선하면 그 진실값은 건드리지 않는다.
        if !statusline_fresh {
            let prev = s.observed_usage.lock().unwrap().clone();
            if let Some(p) = prev {
                let mt = mtime_epoch(std::path::Path::new(&p.session_file));
                if let Some(next) = idle_stale_transition(
                    &p,
                    state.heuristic,
                    now,
                    mt,
                    usage_max_session_age_secs(),
                ) {
                    state.last_discovery = 0.0; // 다음 틱 재발견 강제(가드 본문과 동일 계약)
                    *s.observed_usage.lock().unwrap() = Some(next.clone());
                    daemon.bus.publish(
                        "usage.updated",
                        "usage",
                        Some(s.id),
                        json!({
                            "surface_ref": cys::surface_ref(s.id),
                            "role": s.role.lock().unwrap().clone(),
                            "agent": next.agent, "ctx_pct": next.ctx_pct,
                            "ctx_tokens": next.ctx_tokens, "ctx_window": next.ctx_window,
                            "rate": next.rate, "source": next.source,
                        }),
                    );
                }
            }
        }
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

    // ★B6: 휴리스틱 매핑이 낡았으면 **값을 내지 않는다**(판정 불가를 값으로 위장 금지).
    //   session_file 의 mtime 으로 판정한다 — 그 파일이 이 좌석의 현 세션이라면 방금 쓰였어야
    //   한다. 낡았다는 것은 세션이 교체됐는데 매핑이 따라가지 못했다는 뜻이다(실측 사고).
    //   함께 재발견을 강제해 다음 틱이 lsof(결정론)부터 다시 시도하게 한다.
    if !new.session_file.is_empty() {
        let mt = mtime_epoch(std::path::Path::new(&new.session_file));
        if !mapping_is_fresh(state.heuristic, now, mt, usage_max_session_age_secs()) {
            new.ctx_tokens = None;
            new.ctx_window = None;
            new.ctx_pct = None;
            new.source = format!("{}:stale", new.source);
            state.last_discovery = 0.0; // 다음 틱 재발견 강제(세션 교체 추적)
        }
    }

    // ── 스냅샷 갱신 + 이벤트 (정수 % 변화시에만 — 이벤트 폭주 차단) ──
    let changed = prev
        .as_ref()
        .map(|p| p.ctx_pct != new.ctx_pct || p.rate != new.rate)
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
            }),
        );
    }
    // 결정론 컨텍스트 임계 — 자기보고(status.set)와 **공유 에지 게이트**(ctx_threshold_armed)
    // 로 발화한다. 분리된 에지 상태를 쓰면 같은 교차에 두 경로가 각각 발화해 master/CSO가
    // cycle-agent를 이중 집행한다. payload source:"observed"로 자기보고 발화와 구분.
    if let Some(p) = new.ctx_pct {
        crate::handlers::maybe_fire_context_threshold(daemon, s, p, "observed", Some(&new.agent));
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
                    ext.tails.insert(p.clone(), TailState::attach(daemon, p, false, now));
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

// ─── ★B6(0.14.30): 휴리스틱 매핑 신선도 가드 — 낡은 세션 파일을 값으로 위장하지 않는다 ───
//
// 【실측 사고 · CEO 2026-09-04 04:0x】 본부 reviewer-codex 의 usage 가
// `source=rollout:heuristic · tok=184,535 · pct=71%` 로 표시되는데 그 `session_file` 의 mtime 은
// **9시간 전**이었고 실제 최신 rollout 은 4분 전이었다 — 데몬이 옛 rollout 에 고정돼 있었다.
// dept-1 은 오차가 더 컸다: 매핑값 197,878 vs 최신 rollout 120,811 = **1.64배 과대**(세션 파일
// 20.9시간 전). 컨텍스트 임계 판정이 이 값을 쓰므로 과대면 작업 중 노드를 불필요하게 clear 하고
// (산출 소실) 과소면 임계 초과를 방치한다 — 어느 방향이든 판정이 무력화된다.
//
// 【왜 '값을 주지 않는다' 가 옳은 처리인가】 이 시스템의 계약은 "측정 불능은 통과가 아니다" 다.
// 낡은 값을 그대로 내보내면 소비자(사이클 판정)는 그것을 **측정된 사실**로 읽는다. 그래서
// 신선도 임계를 넘긴 휴리스틱 매핑은 토큰·퍼센트를 **비우고** source 에 `:stale` 을 달아
// '판정 불가' 를 그대로 드러낸다(rate·session_file 은 관측 사실이므로 보존한다).
//
// 【범위】 이 가드는 **휴리스틱 매핑에만** 건다. 등록 매핑(usage.register)·statusline(서버가
// 직접 보고하는 진실)은 이 경로를 타지 않는다 — claude statusline 분기는 무변경이다(회귀 0).

/// 휴리스틱 세션 파일이 '지금 그 좌석의 것' 이라고 믿을 수 있는 최대 나이(초).
/// 기본 900(15분) · `CYS_USAGE_MAX_SESSION_AGE_SECS` 로 조정 · 0 = 가드 비활성(구동작 복원).
fn usage_max_session_age_secs() -> f64 {
    std::env::var("CYS_USAGE_MAX_SESSION_AGE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900.0)
}

/// 매핑 신선도 순수 판정자 — 시계·파일을 읽지 않는다(입력만으로 판정).
///
/// * `heuristic=false`(등록 매핑) → 언제나 신선 취급: 소유자가 명시한 매핑이라 나이로 부정하지 않는다.
/// * `max_age <= 0` → 가드 비활성(종전 동작).
/// * `mtime` 이 미래(시계 스큐)면 신선으로 본다 — 스큐를 stale 로 접으면 정상 좌석이 침묵한다.
pub(crate) fn mapping_is_fresh(heuristic: bool, now: f64, session_mtime: f64, max_age: f64) -> bool {
    if !heuristic || max_age <= 0.0 {
        return true;
    }
    now - session_mtime <= max_age
}

/// ★R1-blocking-1 낡은 매핑의 **idle 전이**(순수) — 신규 줄이 없을 때도 값을 비운다.
///
/// 왜 필요한가(codex 감사 실측): 세션이 교체되면 옛 rollout 파일에는 더 이상 줄이 붙지 않는다.
/// 즉 "신규 줄 0" 은 stale 의 **정상 증상**인데, `collect_for` 는 그 경우 freshness 검사 전에
/// 반환해 이전 numeric snapshot 이 무기한 남았다 — B6 이 막으려던 바로 그 상태(낡은 값을
/// 측정된 사실로 위장)가 idle 경로로 그대로 통과했다.
///
/// 반환 계약: 비울 것이 있을 때만 `Some(새 스냅샷)`. `None` 인 경우는 넷이다 —
/// ⓐ 신선함 ⓑ 등록 매핑(heuristic=false — 소유자 명시) ⓒ 이미 비워진 stale(멱등 — 매 틱
/// `:stale:stale` 로 자라지 않는다) ⓓ statusline(서버 진실값은 이 경로가 건드리지 않는다).
pub(crate) fn idle_stale_transition(
    prev: &ObservedUsage,
    heuristic: bool,
    now: f64,
    session_mtime: f64,
    max_age: f64,
) -> Option<ObservedUsage> {
    if prev.source == "statusline" || prev.source.ends_with(":stale") {
        return None;
    }
    if mapping_is_fresh(heuristic, now, session_mtime, max_age) {
        return None;
    }
    if prev.ctx_tokens.is_none() && prev.ctx_window.is_none() && prev.ctx_pct.is_none() {
        return None; // 비울 수치가 없다 — 무의미한 이벤트를 내지 않는다
    }
    let mut next = prev.clone();
    next.ctx_tokens = None;
    next.ctx_window = None;
    next.ctx_pct = None;
    next.source = format!("{}:stale", prev.source);
    Some(next)
}

/// ★B6 재발견 필요 판정(순수) — 신선도 가드가 `last_discovery = 0.0` 으로 강제하는 그 판정.
///
/// 왜 함수로 뽑았는가(CEO 요구 2026-09-04): 가드는 stale 을 표기하고 재발견을 **강제한다고
/// 주장**하지만, 그 강제가 실제로 발견 경로를 다시 태우는지는 인라인 조건식이던 동안 검체가
/// 잡을 수 없었다. 그러면 "`:stale` 만 붙고 값은 영영 안 돌아오는" 회귀를 아무도 못 잡는다.
/// 이제 판정이 여기 하나뿐이라 ⓐ 가드가 쓰는 필드와 ⓑ 발견 분기가 읽는 필드가 같음이 코드로
/// 닫히고, 아래 검체가 `last_discovery = 0.0` → 재발견 true 를 직접 고정한다.
///
/// * 파일이 사라졌으면 매핑 종류와 무관하게 재발견한다(등록 매핑도 파일은 사라질 수 있다).
/// * 등록 매핑(`heuristic=false`)은 나이로 재발견하지 않는다 — 소유자가 명시한 고정 매핑이다.
fn needs_rediscovery(path_exists: bool, heuristic: bool, now: f64, last_discovery: f64) -> bool {
    !path_exists || (heuristic && now - last_discovery > REDISCOVER_SECS)
}

fn source_label(base: &str, heuristic: bool) -> String {
    if heuristic {
        format!("{base}:heuristic")
    } else {
        base.into()
    }
}

// ───────────────────────── 세션 파일 발견 ─────────────────────────

/// 에이전트별 세션 파일 발견 (등록 부재 시) — claude: 프로필 스캔 / codex: lsof → 휴리스틱
fn discover_session_file(s: &Arc<Surface>, agent: &str, bin: &str) -> Option<PathBuf> {
    let bin_base = bin.rsplit(['/', '\\']).next().unwrap_or(bin);
    let (agent_pid, agent_cwd) = find_agent_descendant(s.pid, bin_base);
    let cwd = agent_cwd.unwrap_or_else(|| s.cwd.clone());
    match agent {
        "claude" => discover_claude_transcript(&cwd, s.created_at),
        "codex" => agent_pid
            .and_then(discover_codex_rollout_lsof)
            .or_else(|| discover_codex_rollout(&cwd, s.created_at)),
        _ => None,
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

/// claude 휴리스틱: `~/.claude*` 전 프로필의 projects/<munged>/ 에서 pane 생성 이후
/// mtime 최신 .jsonl (심링크 프로필은 canonicalize로 중복 제거)
fn discover_claude_transcript(cwd: &str, created_at: f64) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let comp = claude_project_component(cwd);
    let mut best: Option<(f64, PathBuf)> = None;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for e in std::fs::read_dir(&home).ok()?.flatten() {
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
            if best.as_ref().map(|(b, _)| mt > *b).unwrap_or(true) {
                best = Some((mt, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// codex 결정론: 에이전트 프로세스가 열어둔 rollout 파일 fd를 lsof로 직독 (unix 전용 —
/// 실패·미설치 시 None → 휴리스틱 폴백)
///
/// ★U5(0.14.41): Windows 는 **스폰 0** 으로 조기 반환한다. 동봉 PortableGit·MSYS2 에 lsof 가 없어
/// 원래도 실행 실패(None)였고, PATH 에 lsof.exe 가 있는 기계에서만 콘솔 없는 cysd 가 창 정책 없이
/// 띄워 창이 번쩍였다. 결과는 종전 Windows 기본 동작(None → 휴리스틱 폴백)과 같다.
fn discover_codex_rollout_lsof(pid: u32) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
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

/// codex 휴리스틱: 최근 3개 날짜 디렉터리에서 session_meta.cwd 일치 + pane 생성 이후
/// mtime 최신 rollout
fn discover_codex_rollout(cwd: &str, created_at: f64) -> Option<PathBuf> {
    let base = dirs::home_dir()?.join(".codex").join("sessions");
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
    let mut best: Option<(f64, PathBuf)> = None;
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
            if best.as_ref().map(|(b, _)| mt > *b).unwrap_or(true) {
                best = Some((mt, p));
            }
        }
    }
    best.map(|(_, p)| p)
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

/// ★R3-1 검체 이음매(테스트 전용) — 계통 판독(`clear_lineage_depth`) 결과를 **이 스레드에서만** 대체한다.
/// 켜지 않으면(None) 실제 판독이다. 검체의 발신 pid 는 합성값이라 실제 조상 사슬이 없기 때문이다.
/// (이음매를 governance.rs 가 아니라 여기에 두는 이유: governance·handlers 의 소스핀은 첫 cfg(test) 속성을
/// 프로덕션 경계로 쓴다 — 그 파일 프로덕션 구간에 속성을 두면 경계가 당겨져 기존 핀이 조용히 약해진다.)
#[cfg(test)]
pub(crate) mod clear_lineage_seam {
    use std::cell::Cell;
    thread_local! {
        static OVERRIDE: Cell<Option<Option<usize>>> = const { Cell::new(None) };
        // ★리뷰 F2/F3: 훅 기원 대체값. None(기본) = 참 — 깊이 이음매만 켠 종전 검체들이 '좌석 최상위 claude 의 훅' 을
        //   뜻하도록 둔다. 훅 기원 거부를 보는 검체만 Some(false) 로 바꾼다.
        static HOOK: Cell<Option<bool>> = const { Cell::new(None) };
    }
    pub(crate) fn set(v: Option<Option<usize>>) {
        OVERRIDE.with(|c| c.set(v));
    }
    pub(crate) fn get() -> Option<Option<usize>> {
        OVERRIDE.with(|c| c.get())
    }
    pub(crate) fn set_hook(v: Option<bool>) {
        HOOK.with(|c| c.set(v));
    }
    pub(crate) fn hook() -> bool {
        HOOK.with(|c| c.get()).unwrap_or(true)
    }
}

/// ★R3-1 검체 이음매(테스트 전용) — 킬스위치 env(`CYS_CLEAR_REPIN`) 값을 **이 스레드에서만** 대체한다.
/// 프로세스 env 는 병렬 검체끼리 공유되므로 set_var 로 켜고 끄면 다른 검체를 흔든다.
#[cfg(test)]
pub(crate) mod clear_repin_env_seam {
    use std::cell::RefCell;
    thread_local! {
        static OVERRIDE: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
    }
    pub(crate) fn set(v: Option<Option<String>>) {
        OVERRIDE.with(|c| *c.borrow_mut() = v);
    }
    pub(crate) fn get() -> Option<Option<String>> {
        OVERRIDE.with(|c| c.borrow().clone())
    }
}

/// ★R3-1 계통 판독 진입점 → (에이전트 개수, SessionStart 훅 기원) — 실제 판독은 `governance::caller_lineage`
/// (조건·None 의미는 그 doc).
pub(crate) fn clear_lineage(root_pid: u32, caller_pid: u32, agent_bin: &str) -> Option<(usize, bool)> {
    #[cfg(test)]
    if let Some(v) = clear_lineage_seam::get() {
        return v.map(|d| (d, clear_lineage_seam::hook()));
    }
    crate::governance::caller_lineage(root_pid, caller_pid, agent_bin)
}

/// ★리뷰 F3(0.14.42): 이 등록이 **좌석 최상위 claude 의 SessionStart 훅이 아니라고 증명**되는가 — /clear 연속성 기준
/// (`Surface::repin_anchor`)을 옮기지 않을 등록. 증명 = 판독 성공 **그리고** (에이전트 2개 이상 = 중첩 헬퍼 · 에이전트
/// 1개인데 훅 밖 = 도구 셸의 직접 호출). 판독 불가(None · 윈도우 등)와 에이전트 0개(매처가 좌석 에이전트를 못 본다 —
/// 이 좌석의 /clear 는 어차피 lineage_unverified 다)는 증명이 아니다 → 종전처럼 옮긴다.
pub(crate) fn lineage_proves_not_top_hook(lineage: Option<(usize, bool)>) -> bool {
    matches!(lineage, Some((d, hook)) if d >= 2 || (d == 1 && !hook))
}

/// ★리뷰 F2(0.14.42): /clear 재핀 대상 transcript 가 **새 세션**인가. /clear 는 새 session id 를 만든다 — SessionStart:clear
/// 훅 시점에 그 파일은 아직 없거나(실 CC 2.1.282 `-p /clear` 실측: 훅 시점 부재 · 끝난 뒤 2345 B · 줄 6개) 방금 생긴
/// 작은 파일이다. 오래됐거나 큰 파일(가득 찬 옛 대화)로의 재핀은 재기동을 그 대화로 끌고 가 치명 ② 로 직행한다 —
/// 좌석 안 아무 프로세스가 `cys usage-register --transcript <옛 대화> --source clear` 를 불러도 다른 관문은 모두 지난다.
/// 참 = 부재(NotFound) · 또는 일반 파일이면서 크기 ≤ `CLEAR_FRESH_MAX_BYTES` 이고 생성 시각을 읽을 수 있으면
/// `CLEAR_FRESH_MAX_AGE` 안. 생성 시각을 못 읽는 파일시스템은 크기만 본다. 그 밖(판독 오류·디렉터리·미래 생성 시각이
/// 아닌 오래된 파일)은 거짓 = 재핀 거부(종전 동작).
pub(crate) const CLEAR_FRESH_MAX_BYTES: u64 = 256 * 1024;
pub(crate) const CLEAR_FRESH_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(600);

pub(crate) fn clear_transcript_fresh(path: &Path, now: std::time::SystemTime) -> bool {
    let md = match std::fs::metadata(path) {
        Ok(md) => md,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    if !md.is_file() || md.len() > CLEAR_FRESH_MAX_BYTES {
        return false;
    }
    match md.created() {
        // 미래 생성 시각(시계 조정)은 '오래됨' 증거가 아니다 → 크기 판정만 남긴다.
        Ok(born) => match now.duration_since(born) {
            Ok(age) => age <= CLEAR_FRESH_MAX_AGE,
            Err(_) => true,
        },
        Err(_) => true,
    }
}

/// ★R3-1(0.14.42): /clear 뒤 resume 핀 교체 판정(순수 — 핀).
///
/// 핀(`agent_session_id`)은 수집기가 **1회만** 잡는다(is_none 가드 — mtime 흔들림·같은 cwd 동시세션 오핀 방어).
/// 그래서 /clear 로 새 세션 B 가 생겨도 핀은 옛 대화 A 에 남고, 재기동 복원은 비운 컨텍스트 A 를 되살린다
/// (S27b H2 5/5 · 치명 ② 방향). 교체는 **명시 신호**로만 한다: SessionStart 훅이 `source=clear` 로 보낸 등록 중
/// **좌석 최상위 에이전트의 것으로 증명된 것**. mtime·휴리스틱으로는 바꾸지 않는다(가드의 존재 이유가 그대로 남는다).
///
/// 받는 것은 `clear` 하나다. 뺀 것과 이유:
///   · compact 는 세션 id 를 바꾸지 않는다(compact_boundary 앞뒤 sessionId 동일). 받아도 얻는 것이 없다.
///   · resume 은 중첩 `claude -p --resume X` 가 startup 없이 바로 낸다 — 연속성(ⓐ)이 걸러 주지 못하는 모양이다.
///   · startup(수동 재시작)·codex 좌석은 따라가지 않는다(종전과 같음).
///
/// ★clear 도 그 자체로는 좌석 최상위 신호가 **아니다**(2026-09-25 실측 · CC 2.1.282 · HOME 샌드박스):
///   `claude -p "/clear"` 한 번이 SessionStart 를 **startup(N) → clear(B')** 두 번 낸다(startup 훅이 끝난 뒤
///   clear 훅이 시작 · 둘 다 새 session_id·transcript_path · B'.jsonl 즉시 생성). 좌석 Bash 도구의 자식은
///   CYS_SURFACE_ID·cwd 를 물려받으므로 caller 결박만으로는 헬퍼 세션 B' 가 핀이 된다. 그래서 두 겹을 건다:
///   ⓐ **연속성**(순수 · 싸다): 핀이 있으면 **연속성 기준(직전 등록 — 단 좌석 최상위 훅이 아니라고 증명된 등록은
///      건너뛴 것 · `Surface::repin_anchor`) stem 이 현재 핀**이어야 한다. 좌석 자신의 /clear 는 자기 startup·resume(A)
///      또는 앞선 clear 가 등록한 값에서 이어지고, 중첩 `-p` 는 자기 startup(N) 등록이 먼저 온다. 핀이 없으면(수집기
///      첫 틱 전) 비교 대상이 없으니 받는다 — 수집기가 다음 틱에 등록 경로로 잡을 값과 같다. 기준이 **없는데** 핀이
///      있으면 증명 불가 → 거부(결측은 값이 아니다).
///   ⓒ **새 세션**(파일 메타 1회 · 리뷰 F2): 대상 transcript 가 없거나 방금 생긴 작은 파일이어야 한다
///      (`clear_transcript_fresh`) — 가득 찬 옛 대화로의 재핀은 재기동을 치명 ② 로 끌고 간다 → `not_fresh_session`.
///   ⓑ **계통**(프로세스 표 · 비싸서 맨 끝): 발신에서 좌석 루트까지 조상 사슬에 에이전트 실행이 **정확히
///      하나**(= 좌석의 claude)이고 **그 아래에 SessionStart 훅(`session-start.sh`) 실행**이 있어야 한다. 2 이상 = 중첩 →
///      `nested_agent`. 1 인데 훅 밖(좌석 Bash 도구 셸의 직접 호출 등) → `not_hook_origin`. 0·판독 불가(윈도우 조상
///      단절·argv 미관측) → `lineage_unverified`.
/// 증명 범위(정직 표기): ⓑ 는 argv 문자열 판정이다 — 같은 사용자가 훅 스크립트를 가짜 입력으로 직접 돌리면 지난다.
/// 그 경로로 들일 수 있는 것은 ⓒ 를 지나는 대화(없거나 방금 생긴 작은 파일)뿐이다.
/// 실패 방향: 어느 조건이든 어긋나면 **핀 유지 = 종전 동작**(재개는 옛 대화). 좌석 최상위 훅이 아니라고 증명되지 않은
/// 거부(ⓐ·ⓒ·판독 불가 등) 뒤에는 기준이 새 값으로 넘어가 있으므로 같은 좌석의 다음 clear 도 `discontinuous` 로
/// 거부된다 — 좌석이 다시 resume/startup 등록을 낼 때(재기동)까지 종전 동작이 이어진다(거부가 새 교체를 부르는 방향은 없다).
#[allow(clippy::too_many_arguments)]
pub(crate) fn clear_repin_verdict(
    source: Option<&str>,
    caller_bound: bool,
    agent: Option<&str>,
    current: Option<&str>,
    prev_registered: Option<&Path>,
    transcript: &Path,
    held_by_other_seat: impl FnOnce(&str) -> bool,
    transcript_fresh: impl FnOnce() -> bool,
    lineage: impl FnOnce() -> Option<(usize, bool)>,
) -> Result<String, &'static str> {
    if source != Some("clear") {
        return Err("not_clear");
    }
    if agent != Some("claude") {
        return Err("not_claude");
    }
    // 좌석 결박: 발신이 **이 좌석의 자손**으로 해석될 때만(익명·좌석 밖 호출은 위조와 구별할 수 없다).
    if !caller_bound {
        return Err("caller_unbound");
    }
    let Some(sid) = extract_session_id("claude", transcript) else {
        return Err("bad_session_id");
    };
    // 복원은 id 를 `--resume {session_id}` 로 기동 문자열에 인라인한다 — 셸 메타문자가 든 stem 을 핀으로
    // 들이지 않는다(claude 세션 id = UUID).
    if !is_plausible_session_id(&sid) {
        return Err("bad_session_id");
    }
    if current == Some(sid.as_str()) {
        return Err("unchanged");
    }
    // ⓐ 연속성 — None 은 값이 아니다: 핀(Some) 과 직전 등록(None) 을 같다고 보지 않는다.
    if let Some(cur) = current {
        let prev = prev_registered.and_then(|p| extract_session_id("claude", p));
        if prev.as_deref() != Some(cur) {
            return Err("discontinuous");
        }
    }
    // 다른 좌석이 이미 쥔 세션이면 교체하지 않는다(두 좌석이 한 대화로 복원되는 분열 방지).
    if held_by_other_seat(&sid) {
        return Err("held_by_other_seat");
    }
    // ⓒ 새 세션 — /clear 는 새 session id 를 만든다. 오래됐거나 큰 파일(옛 대화)이면 받지 않는다.
    if !transcript_fresh() {
        return Err("not_fresh_session");
    }
    // ⓑ 계통 — 프로세스 표를 읽으므로 싼 조건을 모두 지난 뒤에만 부른다.
    match lineage() {
        Some((1, true)) => Ok(sid),
        Some((1, false)) => Err("not_hook_origin"),
        Some((0, _)) | None => Err("lineage_unverified"),
        Some(_) => Err("nested_agent"),
    }
}

/// 세션 id 형태(1..=128자 · ASCII 영숫자·`-`·`_`). claude 는 UUID 다.
pub(crate) fn is_plausible_session_id(s: &str) -> bool {
    (1..=128).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// ★R3-1 롤백 ①: 데몬 env `CYS_CLEAR_REPIN=0` 이면 clear 재핀 블록 **전체**를 건너뛴다 — 판정·계통 판독·
/// persist·이벤트가 모두 0 이고 등록 자체는 종전대로다(= v0.14.41 동작). 그 밖 값·부재는 켜짐(엄격 `"0"` 비교).
/// 매 clear 등록마다 읽는다(데몬 env 는 재시작 없이는 바뀌지 않는다 — 롤백 순서는 설계서: 훅 되돌리기가 1순위).
pub(crate) fn clear_repin_enabled() -> bool {
    #[cfg(test)]
    if let Some(v) = clear_repin_env_seam::get() {
        return clear_repin_enabled_from(v.as_deref());
    }
    clear_repin_enabled_from(std::env::var("CYS_CLEAR_REPIN").ok().as_deref())
}

/// 순수 코어(진리표 대상).
pub(crate) fn clear_repin_enabled_from(v: Option<&str>) -> bool {
    v != Some("0")
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
// agy는 토큰·쿼터를 평문 로컬 파일에 안 남긴다 — 실행 중 프로세스의 로컬 LS RPC(HTTPS, self-signed)로만
// 노출된다. 포트는 매 실행 변동 → agy 로그의 언어 서버 줄(없으면 lsof)로 발견·probe로 검증·캐시. 파일 tail
// 수집기와 분리된 저빈도 비동기 태스크(async curl — tokio 워커 미블로킹). HTTP 클라이언트 의존성을 더하지
// 않으려 curl 셸아웃을 쓴다(codex의 lsof 셸아웃과 동형). 실패·미설치는 graceful(배지 없음 유지).
// ★0.14.42(2026-09-23): 2026-06-17 첫 실측 때는 127.0.0.1 무인증이었으나 **지금은 CSRF 필수로 확정**됐다
//   (오너 승인 라이브 프로브 3회 · 22:11–22:15 PDT · 언어 서버 1.2.9: 헤더 없는 이 요청 → HTTP 401
//   `{"code":"unauthenticated","message":"missing CSRF token"}` · `x-codeium-csrf-token` 에 틀린 값 → `invalid CSRF
//   token` · 평문 HTTP 포트도 401). 토큰은 agy 인자·환경변수 어디에도 없다(언어 서버가 agy 프로세스 안에서 돈다).
//   cys 는 토큰을 찾아 읽지 않는다(그 토큰은 쿼터만이 아니라 로컬 agy API 전체를 연다 · 오너 승인 밖).
//   → 값의 **주 경로는 agy 공식 상태줄(statusLine) 훅**이다(cys.rs `agy_statusline_to_report_params` →
//   usage.report · 계정 source "agy-statusline"). 이 RPC 경로는 진단용으로 남되, CSRF 거절은 전용 코드
//   `agy_csrf_required` 로 분류하고 agy pid 마다 [`AGY_CSRF_BACKOFF_SECS`] 동안 다시 두드리지 않는다(종전: 좌석마다
//   15초마다 같은 401). 상태줄 값이 한 번 들어오면 수집기는 프로브를 멈춘다(`accounts::agy_statusline_authoritative`).

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

/// agy 가 LISTEN 하는 TCP 포트를 묻는 lsof 인자(순수 — 핀 테스트).
///
/// ★`-a` 필수(0.14.42 RC2-a): lsof 는 선택 조건(`-p`·`-i`)을 기본 **OR** 로 합친다. 종전 인자에는 `-a` 가
/// 없어 "이 pid 의 파일 **또는** 기계 전체의 LISTEN 소켓"이 나왔고, 12개 상한과 겹쳐 각 데몬이 자기 agy 가
/// 아니라 Discord·aside-daemon·pid 가 가장 작은 (다른 부서의) agy 포트를 두드렸다(2026-09-23 실측).
/// ★(0.14.42 · R4-03) `-b`(stat·lstat·readlink 처럼 막힐 수 있는 커널 호출 회피)·`-w`(그로 인한 경고 억제) — agy 가 멈춘
/// 네트워크 마운트의 파일을 쥐고 있어도 lsof 가 stat 에서 서지 않는다(이 맥 실측: -b -w 유무로 -Fn 출력 동일). 호출부는 따로
/// [`AGY_LSOF_TIMEOUT`] 로 감싼다.
fn agy_lsof_listen_args(pid: u32) -> Vec<String> {
    let pid = pid.to_string();
    ["-b", "-w", "-nP", "-a", "-p", pid.as_str(), "-iTCP", "-sTCP:LISTEN", "-Fn"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// agy 가 연 파일 목록을 묻는 lsof 인자 — 자기 로그 파일을 찾는다(선택 조건이 하나라 OR 문제는 없다).
fn agy_lsof_files_args(pid: u32) -> Vec<String> {
    let pid = pid.to_string();
    ["-b", "-w", "-nP", "-a", "-p", pid.as_str(), "-Fn"].iter().map(|s| s.to_string()).collect()
}

/// ★(0.14.42 · R4-03) agy lsof 한 번의 시간 상한 — 같은 파일의 curl 프로브(3s)와 같은 규율. 수집기 루프는 틱 하나가 끝나야
/// 다음 틱으로 가므로, 상한 없는 await 하나가 멈추면 agy 쿼터 관측 전체가 조용히 끊긴다.
const AGY_LSOF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// agy lsof 실행(시간 상한 · 초과면 자식 kill · 실패·초과 = None → 호출부의 '포트 없음' 폴백).
async fn agy_lsof_output(args: Vec<String>) -> Option<std::process::Output> {
    let fut = tokio::process::Command::new("lsof").args(args).kill_on_drop(true).output();
    tokio::time::timeout(AGY_LSOF_TIMEOUT, fut).await.ok()?.ok()
}

/// agy 로그의 언어 서버 줄 머리말(agy 1.1.x `server.go`). 2026-09-23 실측: 로그 4개 모두 첫 ~300바이트에
/// `… Language server listening on random port at <N> for HTTPS (gRPC)` 와 바로 아래 `<N+1> for HTTP` 가 있다.
const AGY_LS_LINE: &str = "Language server listening on random port at ";
/// 로그에서 읽는 머리 상한 — 줄이 첫머리에 있으므로 수 MB 로그 전체를 15초마다 읽지 않는다.
const AGY_LOG_HEAD_BYTES: u64 = 16 * 1024;

/// agy 로그 본문 → 언어 서버 **HTTPS** 포트(순수 — 핀). 줄이 여럿이면(재기동) 마지막 것. HTTP 줄·숫자 아님·
/// u16 범위 밖은 버린다(추측 금지).
pub fn parse_agy_ls_https_port(log_text: &str) -> Option<u16> {
    let mut last = None;
    for line in log_text.lines() {
        let Some(i) = line.find(AGY_LS_LINE) else {
            continue;
        };
        let rest = &line[i + AGY_LS_LINE.len()..];
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let (num, tail) = rest.split_at(end);
        let proto = tail
            .trim_start()
            .strip_prefix("for ")
            .and_then(|t| t.split_whitespace().next())
            .unwrap_or("");
        if proto != "HTTPS" {
            continue;
        }
        if let Ok(p) = num.parse::<u16>() {
            if p > 0 {
                last = Some(p);
            }
        }
    }
    last
}

/// `lsof -Fn` 출력 → agy 자신의 로그 파일(`…/antigravity-cli/log/cli-*.log`). 토큰 파일·심볼릭 `cli.log`
/// (`log/` 밖)는 고르지 않는다. 순수 — 핀.
fn agy_log_path_from_lsof(lsof_fn: &str) -> Option<PathBuf> {
    lsof_fn
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter(|n| n.contains("/antigravity-cli/log/") && n.ends_with(".log"))
        .last()
        .map(PathBuf::from)
}

/// agy 가 연 로그에서 언어 서버 HTTPS 포트를 읽는다 — 포트를 **추측하지 않고** agy 가 스스로 적은 값을 쓴다.
async fn agy_ls_port_from_log(pid: u32) -> Option<u16> {
    if cfg!(windows) {
        return None; // lsof 부재(agy_listen_ports 와 같은 이유 · 스폰 0)
    }
    let out = agy_lsof_output(agy_lsof_files_args(pid)).await?;
    let path = agy_log_path_from_lsof(&String::from_utf8_lossy(&out.stdout))?;
    let mut head = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .take(AGY_LOG_HEAD_BYTES)
        .read_to_end(&mut head)
        .ok()?;
    parse_agy_ls_https_port(&String::from_utf8_lossy(&head))
}

/// 한 포트 프로브의 분류 결과. 실패도 **종류별로** 남긴다 — 종전엔 전부 조용히 None 이라 "경로 고장"과
/// "아직 관측 전"이 화면에서 구별되지 않았다(0.14.42 RC2).
#[derive(Debug, Clone, PartialEq)]
pub enum AgyProbe {
    /// 200 + Gemini 쿼터 그룹.
    Ok(Vec<RateWindow>),
    /// 언어 서버가 HTTP 로 답했지만 성공이 아니다(CSRF 가 아닌 거절). 코드 보존.
    Http(u16),
    /// 언어 서버가 CSRF 토큰을 요구하며 거절했다(4xx + 본문 "CSRF token" · 2026-09-23 실측 401). 결정론적 거절이라
    /// 같은 agy 에 다시 물어도 같은 답이다 → 백오프 대상.
    CsrfRequired,
    /// 200 인데 Gemini 쿼터가 없다(스키마 드리프트).
    NoQuota,
    /// 연결·TLS·시간 초과 — 그 포트에 언어 서버가 없다.
    Unreachable,
}

/// curl 결과 → 분류(순수 — 핀). stdout 은 `본문 + "\n" + HTTP 코드`(`-w "\n%{http_code}"`) 형태다.
fn classify_agy_probe(curl_ok: bool, stdout: &[u8]) -> AgyProbe {
    if !curl_ok {
        return AgyProbe::Unreachable;
    }
    let text = String::from_utf8_lossy(stdout);
    let Some(nl) = text.rfind('\n') else {
        return AgyProbe::Unreachable;
    };
    let (body, code) = (&text[..nl], text[nl + 1..].trim());
    let Ok(code) = code.parse::<u16>() else {
        return AgyProbe::Unreachable;
    };
    if code == 0 {
        return AgyProbe::Unreachable; // curl "000" = HTTP 응답 자체가 없다
    }
    if code != 200 {
        // ★0.14.42 RC2-b: CSRF 거절(4xx + 본문 "CSRF token" — 실측 원문 `missing CSRF token`·`invalid CSRF token`)은
        //   전용 분류다. 판별은 본문 문구가 한다(코드는 401 실측이나 403 으로 바뀌어도 뜻은 같다). 5xx 는 서버 오류.
        if (400..500).contains(&code) && body.to_ascii_lowercase().contains("csrf token") {
            return AgyProbe::CsrfRequired;
        }
        return AgyProbe::Http(code);
    }
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return AgyProbe::NoQuota;
    };
    let rate = parse_agy_quota(&v);
    if rate.is_empty() {
        AgyProbe::NoQuota
    } else {
        AgyProbe::Ok(rate)
    }
}

/// 계정 행 `source_error` 코드(안정 문자열 — UI 가 사람 말로 옮긴다). 성공은 코드 없음.
fn agy_error_code(p: &AgyProbe) -> Option<String> {
    match p {
        AgyProbe::Ok(_) => None,
        AgyProbe::Http(c) => Some(format!("agy_http_{c}")),
        AgyProbe::CsrfRequired => Some(AGY_ERR_CSRF.into()),
        AgyProbe::NoQuota => Some("agy_no_quota".into()),
        AgyProbe::Unreachable => Some("agy_unreachable".into()),
    }
}
/// 언어 서버가 CSRF 토큰을 요구한다(agy 1.2.x) — 값은 agy 상태줄 훅으로만 받을 수 있다(UI: "agy 상태줄 연결 필요").
pub const AGY_ERR_CSRF: &str = "agy_csrf_required";
/// CSRF 거절을 받은 agy pid 를 다시 두드리지 않는 시간(초). agy 가 재기동(=새 pid · 업데이트 포함)하면 즉시 다시 묻는다.
const AGY_CSRF_BACKOFF_SECS: f64 = 1800.0;

/// 이 agy pid 가 CSRF 백오프 중인가(순수 — 핀).
fn agy_csrf_backoff_active(backoff: &HashMap<u32, f64>, pid: u32, now: f64) -> bool {
    backoff.get(&pid).is_some_and(|until| now < *until)
}
/// 이 플랫폼(Windows)에서는 언어 서버 RPC 경로가 성립하지 않는다 — 값은 agy 상태줄 훅으로만 받는다(UI: "agy 상태줄
/// 연결 필요"). fatal-fix W5.
pub const AGY_ERR_STATUSLINE_REQUIRED: &str = "agy_statusline_required";
/// agy 좌석은 있는데 그 아래 agy 프로세스를 못 찾았다.
const AGY_ERR_NO_PROCESS: &str = "agy_no_process";
/// agy 는 찾았는데 물어볼 포트가 하나도 없다.
const AGY_ERR_NO_PORT: &str = "agy_no_port";

/// 한 틱에 좌석·포트마다 실패 이유가 다르면 **가장 멀리 간** 실패를 적는다 — 언어 서버가 직접 답한 거부가
/// 가장 구체적인 사실이다.
fn agy_error_rank(code: &str) -> u8 {
    if code == AGY_ERR_CSRF {
        5 // 거절 사유까지 안다 — 값을 얻는 길(상태줄 훅)을 가리키는 가장 구체적인 사실
    } else if code.starts_with("agy_http_") || code == "agy_no_quota" {
        4
    } else if code == "agy_unreachable" {
        3
    } else if code == AGY_ERR_NO_PORT {
        2
    } else {
        1
    }
}

fn keep_worse(best: &mut Option<String>, code: String) {
    if best.as_deref().map_or(true, |b| agy_error_rank(&code) > agy_error_rank(b)) {
        *best = Some(code);
    }
}

/// agy 프로세스가 LISTEN하는 127.0.0.1/localhost 포트 목록 (lsof — codex 패턴 동형, 와일드카드 제외).
///
/// ★U5(0.14.41): Windows 는 **스폰 0** 으로 조기 반환한다 — lsof 가 없어 원래도 빈 목록이었고
/// (= Windows 의 agy 쿼터 수집은 종전부터 불능), lsof.exe 가 PATH 에 있는 기계에서만 콘솔 없는
/// cysd 가 15초마다 창 정책 없이 띄워 창이 번쩍였다. 결과는 종전 Windows 기본 동작과 같다.
async fn agy_listen_ports(pid: u32) -> Vec<u16> {
    if cfg!(windows) {
        return Vec::new();
    }
    let Some(out) = agy_lsof_output(agy_lsof_listen_args(pid)).await else {
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
/// 결과는 분류해 돌려준다(성공·거부 코드·쿼터 없음·도달 불가) — 실패를 삼키지 않는다.
async fn agy_quota_probe(port: u16) -> AgyProbe {
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
            // 본문 뒤에 HTTP 코드를 한 줄 덧붙인다 — 거부(401/403 등)를 '도달 불가'와 구별하려고(RC2).
            "-w",
            "\n%{http_code}",
            // R-CLI-3(부차): URL이 고정 localhost(포트 숫자)라 실위험은 없으나 동형 패턴 방어심층 —
            // `--` 옵션 종결자로 URL을 위치 인자로 강제한다.
            "--",
            &url,
        ])
        // Windows: 주기 프로브가 콘솔 창을 반복 플래시하지 않게(콘솔 없는 cysd의 콘솔 자식).
        .hide_console()
        .output();
    match tokio::time::timeout(Duration::from_secs(3), fut).await {
        Ok(Ok(out)) => classify_agy_probe(out.status.success(), &out.stdout),
        _ => AgyProbe::Unreachable,
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

/// 한 agy surface의 쿼터 수집. 순서: ① 직전 성공 포트(캐시) → ② agy 가 연 로그의 언어 서버 HTTPS 포트(결정론)
/// → ③ 폴백: 이 agy 가 LISTEN 하는 localhost 포트(`-a` 로 AND · 상한 12). 실패면 가장 구체적인 오류 코드.
async fn collect_agy_for(
    daemon: &Arc<Daemon>,
    s: &Arc<Surface>,
    ports: &mut HashMap<u64, u16>,
    csrf_backoff: &mut HashMap<u32, f64>,
) -> Result<(), String> {
    let mut best: Option<String> = None;
    if let Some(p) = ports.get(&s.id).copied() {
        match agy_quota_probe(p).await {
            AgyProbe::Ok(rate) => {
                update_agy_usage(daemon, s, rate);
                return Ok(());
            }
            other => {
                ports.remove(&s.id); // 캐시 무효화 — 아래에서 재발견
                if let Some(c) = agy_error_code(&other) {
                    keep_worse(&mut best, c);
                }
            }
        }
    }
    let (agy_pid, _) = find_agent_descendant(s.pid, "agy");
    let Some(pid) = agy_pid else {
        return Err(best.unwrap_or_else(|| AGY_ERR_NO_PROCESS.into()));
    };
    // ★RC2-b: 이 agy 가 CSRF 로 거절한 지 얼마 안 됐다 — lsof·로그 읽기·curl 없이 같은 사실을 다시 적는다.
    if agy_csrf_backoff_active(csrf_backoff, pid, now_epoch()) {
        keep_worse(&mut best, AGY_ERR_CSRF.into());
        return Err(best.unwrap_or_else(|| AGY_ERR_CSRF.into()));
    }
    let log_port = agy_ls_port_from_log(pid).await;
    let mut candidates: Vec<u16> = log_port.into_iter().collect();
    for p in agy_listen_ports(pid).await {
        if !candidates.contains(&p) {
            candidates.push(p);
        }
    }
    if candidates.is_empty() {
        return Err(best.unwrap_or_else(|| AGY_ERR_NO_PORT.into()));
    }
    for port in candidates {
        let r = agy_quota_probe(port).await;
        if let AgyProbe::Ok(rate) = r {
            ports.insert(s.id, port);
            update_agy_usage(daemon, s, rate);
            return Ok(());
        }
        let csrf = r == AgyProbe::CsrfRequired;
        if csrf {
            csrf_backoff.insert(pid, now_epoch() + AGY_CSRF_BACKOFF_SECS);
        }
        let reached_ls = matches!(r, AgyProbe::Http(_) | AgyProbe::NoQuota | AgyProbe::CsrfRequired);
        if let Some(c) = agy_error_code(&r) {
            keep_worse(&mut best, c);
        }
        // agy 가 스스로 적은 언어 서버 포트가 HTTP 로 답했다 = 언어 서버는 찾았다. 나머지 포트(같은 agy 의 다른
        // 리스너)를 더 두드려도 쿼터 서비스가 아니다 — 소음만 늘린다. CSRF 거절은 어느 포트에서 왔든 언어 서버다.
        if reached_ls && (Some(port) == log_port || csrf) {
            break;
        }
    }
    Err(best.unwrap_or_else(|| "agy_unreachable".into()))
}

/// agy(Antigravity) 쿼터 수집기 — 파일 tail과 분리된 저빈도 비동기 태스크.
/// 틱마다 결과를 계정 행에 정직하게 남긴다: 한 좌석이라도 성공 → (note_rate 가 오류를 지움) · 전부 실패 →
/// 가장 구체적인 오류 코드 · agy 좌석 0 → 오류 지움(좌석이 없는 것은 고장이 아니다).
pub fn spawn_agy_collector(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut ports: HashMap<u64, u16> = HashMap::new();
        // agy pid → CSRF 백오프 만료 시각(RC2-b). 만료분은 틱마다 걷는다(크기 = 30분 안에 본 agy 수).
        let mut csrf_backoff: HashMap<u32, f64> = HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(agy_poll_secs())).await;
            agy_collector_tick(&daemon, &mut ports, &mut csrf_backoff, agy_rpc_supported()).await;
        }
    });
}

/// 이 플랫폼에서 agy 언어 서버 RPC 경로가 성립하는가 — Windows 는 포트를 찾을 길(lsof·agy 로그의 포트 줄)이 없다.
fn agy_rpc_supported() -> bool {
    !cfg!(windows)
}

/// 수집기 한 틱(시험 이음매 — `rpc_supported` 로 플랫폼을 주입한다).
async fn agy_collector_tick(
    daemon: &Arc<Daemon>,
    ports: &mut HashMap<u64, u16>,
    csrf_backoff: &mut HashMap<u32, f64>,
    rpc_supported: bool,
) {
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
    let now = now_epoch();
    csrf_backoff.retain(|_, until| *until > now);
    // ★fatal-fix R4-F1: 이 태스크는 async 문맥이다 — 전역 accounts **표준 뮤텍스를 기다리지 않는다**(try 판).
    //   기다리면 tokio 워커가 붙잡히고, 그 워커가 IO 드라이버를 돌리던 것이면 데몬의 모든 소켓 요청(ping·GUI 입력·
    //   훅)이 멈춘다(워커 1개 런타임은 영구 정지). 경합이면 이 틱을 건너뛴다 — 값은 다음 틱에 다시 적힌다.
    if surfaces.is_empty() {
        let _ = crate::accounts::try_note_agy_error(daemon, None);
        return;
    }
    // ★RC2-b: agy 상태줄 훅이 값을 보내고 있으면(계정의 최신 출처) 이 경로는 물러선다 — CSRF 로 막힌
    //   RPC 를 계속 두드려 값 있는 행에 '관측 실패'를 덧씌우지 않는다(오래됨은 stale 표기가 따로 말한다).
    match crate::accounts::try_agy_statusline_authoritative(daemon) {
        Some(false) => {}
        Some(true) | None => return,
    }
    // ★fatal-fix W5: 언어 서버 포트를 찾을 길이 없는 플랫폼(Windows)은 프로브하지 않는다 — 값을 얻는 길(상태줄)을 적는다.
    if !rpc_supported {
        let _ = crate::accounts::try_note_agy_error(daemon, Some(AGY_ERR_STATUSLINE_REQUIRED));
        return;
    }
    let mut any_ok = false;
    let mut worst: Option<String> = None;
    for s in &surfaces {
        match collect_agy_for(daemon, s, ports, csrf_backoff).await {
            Ok(()) => any_ok = true,
            Err(code) => keep_worse(&mut worst, code),
        }
    }
    if !any_ok {
        let _ = crate::accounts::try_note_agy_error(daemon, worst.as_deref());
    }
}

#[cfg(test)]
mod tests {

    // ─────────── ★B6(0.14.30): 휴리스틱 매핑 신선도 가드 핀(b6_*) ───────────

    use super::{mapping_is_fresh, usage_max_session_age_secs};

    /// 낡은 세션 파일은 **값의 근거가 아니다** — 실측 사고(9시간·20.9시간 전 매핑에서 1.64배
    /// 과대)를 재현하는 나이에서 stale 로 떨어져야 한다.
    #[test]
    fn b6_stale_heuristic_mapping_is_not_fresh() {
        let now = 1_000_000.0;
        // 9시간 전(본부 실측) · 20.9시간 전(dept-1 실측) 둘 다 stale.
        assert!(!mapping_is_fresh(true, now, now - 9.0 * 3600.0, 900.0));
        assert!(!mapping_is_fresh(true, now, now - 20.9 * 3600.0, 900.0));
        // 임계 직전은 신선(경계 포함).
        assert!(mapping_is_fresh(true, now, now - 900.0, 900.0));
        assert!(mapping_is_fresh(true, now, now - 60.0, 900.0));
    }

    /// 등록 매핑(usage.register)·가드 비활성은 나이로 부정하지 않는다 — 이 가드는 **휴리스틱
    /// 전용**이고, claude statusline 경로는 애초에 이 판정을 타지 않는다(회귀 0).
    #[test]
    fn b6_registered_mapping_and_disabled_knob_are_always_fresh() {
        let now = 1_000_000.0;
        assert!(
            mapping_is_fresh(false, now, now - 48.0 * 3600.0, 900.0),
            "등록 매핑은 소유자가 명시한 것이라 나이로 부정하지 않는다"
        );
        assert!(
            mapping_is_fresh(true, now, now - 48.0 * 3600.0, 0.0),
            "임계 0 = 가드 비활성 = 종전 동작(즉시 복원 스위치)"
        );
    }

    /// 시계 스큐(미래 mtime)를 stale 로 접으면 정상 좌석이 침묵한다 — 신선으로 본다.
    #[test]
    fn b6_future_mtime_from_clock_skew_is_treated_as_fresh() {
        let now = 1_000_000.0;
        assert!(mapping_is_fresh(true, now, now + 120.0, 900.0));
    }

    /// 기본 임계는 15분이다(설정 가능) — 기본값이 곧 계약이므로 상수를 핀한다.
    #[test]
    fn b6_default_max_session_age_is_fifteen_minutes() {
        let prev = std::env::var("CYS_USAGE_MAX_SESSION_AGE_SECS").ok();
        std::env::remove_var("CYS_USAGE_MAX_SESSION_AGE_SECS");
        let got = usage_max_session_age_secs();
        match prev {
            Some(v) => std::env::set_var("CYS_USAGE_MAX_SESSION_AGE_SECS", v),
            None => std::env::remove_var("CYS_USAGE_MAX_SESSION_AGE_SECS"),
        }
        assert_eq!(got, 900.0);
    }

    /// 실파일 픽스처 — mtime 을 실제로 읽어 판정한다(판정자 단독 단위 테스트의 사각지대인
    /// "파일에서 시각을 못 읽으면 어떻게 되나"를 포함해 고정한다).
    #[test]
    fn b6_file_fixtures_are_classified_by_real_mtime() {
        let dir = std::env::temp_dir().join(format!("cys-b6-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let fresh = dir.join("rollout-fresh.jsonl");
        std::fs::write(&fresh, b"{}\n").expect("write");
        let mt = mtime_epoch(&fresh);
        let now = crate::state::now_epoch();
        assert!(
            mapping_is_fresh(true, now, mt, 900.0),
            "방금 쓴 파일이 stale 로 떨어지면 정상 좌석이 통째로 침묵한다"
        );
        // 같은 파일을 '10시간 뒤 시점' 에서 보면 stale — 실측 사고(9시간·20.9시간)의 재현.
        assert!(!mapping_is_fresh(true, now + 10.0 * 3600.0, mt, 900.0));
        // 없는 파일: mtime_epoch 이 0 을 내므로 stale 로 떨어진다(값 미제공 = 안전 방향).
        let missing = dir.join("nope.jsonl");
        assert!(!mapping_is_fresh(true, now, mtime_epoch(&missing), 900.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★R1-blocking-1 (codex 감사 · 실패 먼저 잠금): **신규 줄이 없어도** 낡은 매핑은 값을
    /// 비워야 한다. 세션이 교체되면 옛 파일에는 더 이상 줄이 붙지 않으므로 "신규 줄 0" 은
    /// stale 의 **정상 증상**이다 — 그런데 collect_for 는 그 경우 freshness 검사 **전에**
    /// 반환해 이전 numeric snapshot 이 무기한 남았다. 내 기존 B6 검체는 판정자·수동 전이만
    /// 봐서 이 조기 반환을 못 봤다(검체가 축을 안 보고 있었다).
    ///
    /// 이 검체는 그 축을 직접 잰다: 이전 스냅샷(수치 보유) + 오래된 mtime + 신규 줄 0.
    #[test]
    fn b6_idle_stale_clears_previous_snapshot_without_new_lines() {
        let prev = ObservedUsage {
            agent: "codex".into(),
            ctx_tokens: Some(197_878),
            ctx_window: Some(258_400),
            ctx_pct: Some(76),
            rate: Vec::new(),
            source: "rollout:heuristic".into(),
            session_file: "/x/rollout-old.jsonl".into(),
            updated_at: 1.0,
        };
        let now = crate::state::now_epoch();
        let old_mt = now - 20.9 * 3600.0; // dept-1 실측(20.9시간 정지)
        // ⓐ 낡음 + 이전 수치 보유 → 비운 스냅샷을 낸다.
        let got = idle_stale_transition(&prev, true, now, old_mt, 900.0)
            .expect("낡은 매핑인데 전이가 없다 — 이전 수치가 무기한 남는다");
        assert!(got.ctx_tokens.is_none() && got.ctx_window.is_none() && got.ctx_pct.is_none());
        assert_eq!(got.source, "rollout:heuristic:stale", "provenance 에 stale 이 남아야 한다");
        assert_eq!(got.session_file, prev.session_file, "어느 파일이 낡았는지는 보존");
        // ⓑ 음성 대조 — 신선하면 전이 없음(무조건 비우는 구현 차단).
        assert!(idle_stale_transition(&prev, true, now, now - 10.0, 900.0).is_none());
        // ⓒ 음성 대조 — 등록 매핑(heuristic=false)은 나이로 비우지 않는다.
        assert!(idle_stale_transition(&prev, false, now, old_mt, 900.0).is_none());
        // ⓓ 멱등 — 이미 비워진 stale 스냅샷은 매 틱 다시 전이하지 않는다(:stale:stale 방지).
        assert!(idle_stale_transition(&got, true, now, old_mt, 900.0).is_none());
        // ⓔ statusline 진실값은 이 경로가 건드리지 않는다.
        let mut sl = prev.clone();
        sl.source = "statusline".into();
        assert!(idle_stale_transition(&sl, true, now, old_mt, 900.0).is_none());
    }

    /// ★생산 배선 핀(#4 회귀 0): 가드는 ⓐ statusline 조기 반환 **뒤**에 있고 ⓑ 세 수치를
    /// 비우며 ⓒ `:stale` 을 붙이고 ⓓ 재발견을 강제한다. claude statusline 경로는 이 지점에
    /// 도달하지 않으므로 무영향이다(그 순서가 깨지면 claude 값이 지워질 수 있다).
    #[test]
    fn b6_guard_sits_after_statusline_return_and_clears_numbers() {
        let src = include_str!("usage.rs");
        let body = src
            .split("fn collect_for(")
            .nth(1)
            .expect("collect_for 소실");
        let sl = body.find("if statusline_fresh {").expect("statusline 조기 반환 소실");
        let guard = body.find("mapping_is_fresh(").expect("신선도 가드 소실");
        assert!(
            sl < guard,
            "가드가 statusline 조기 반환보다 앞에 있으면 claude 서버 진실값이 지워진다"
        );
        let tail = &body[guard..guard + 600.min(body.len() - guard)];
        for needle in [
            "ctx_tokens = None",
            "ctx_window = None",
            "ctx_pct = None",
            ":stale",
            "last_discovery = 0.0",
        ] {
            assert!(tail.contains(needle), "가드 계약 누락: {needle}");
        }
        // ★R1-blocking-1 배선 핀: idle 전이는 `lines.is_empty()` **반환 안**에서 불려야 한다.
        //   순수 함수 검체만으로는 호출부가 사라져도 초록이라(codex 가 지적한 '축을 안 보는
        //   검체' 재발) 여기서 호출 위치를 함께 잠근다.
        let empty_ret = body.find("if lines.is_empty()").expect("무신규라인 분기 소실");
        let idle_call = body.find("idle_stale_transition(").expect("idle 전이 호출 소실");
        assert!(
            empty_ret < idle_call && idle_call < sl,
            "idle 전이가 무신규라인 분기 안(그리고 statusline 반환 앞)에 없다 — \
             낡은 수치가 무기한 남는 경로가 다시 열린다"
        );
    }

    /// 소비자 계약 핀: stale 표기는 `:stale` 접미로 드러나고 값은 비어 있다(판정 불가를 값으로
    /// 위장하지 않는다). 여기서는 그 조립 규칙 자체를 고정한다.
    #[test]
    fn b6_stale_snapshot_carries_no_numbers_but_keeps_provenance() {
        let mut u = ObservedUsage {
            agent: "codex".into(),
            ctx_tokens: Some(184_535),
            ctx_window: Some(258_400),
            ctx_pct: Some(71),
            rate: Vec::new(),
            source: "rollout:heuristic".into(),
            session_file: "/x/rollout-old.jsonl".into(),
            updated_at: 1.0,
        };
        // 생산 코드와 같은 전이(값 비우기 + :stale 표기).
        u.ctx_tokens = None;
        u.ctx_window = None;
        u.ctx_pct = None;
        u.source = format!("{}:stale", u.source);
        assert_eq!(u.source, "rollout:heuristic:stale");
        assert!(u.ctx_tokens.is_none() && u.ctx_pct.is_none());
        assert_eq!(
            u.session_file, "/x/rollout-old.jsonl",
            "어느 파일이 낡았는지는 진단에 필요한 사실이라 보존한다"
        );
    }

    /// ★CEO 요구 증거(2026-09-04): stale 이 붙은 뒤 **재발견이 실제로 다시 돈다**.
    ///
    /// 가드는 `state.last_discovery = 0.0` 을 쓰는데, 그 쓰기가 발견 분기를 다시 태우지
    /// 못하면 좌석은 `:stale` 만 단 채 값이 영영 돌아오지 않는다(무음 영구 침묵). 여기서
    /// 가드가 쓰는 값 그대로를 판정자에 넣어 재발견 true 를 고정한다.
    #[test]
    fn b6_stale_reset_forces_rediscovery_next_tick() {
        let now = 1_700_000_000.0;
        // 가드가 남긴 상태: 파일은 아직 있고, 휴리스틱이며, last_discovery 는 0.0.
        assert!(
            needs_rediscovery(true, true, now, 0.0),
            "가드의 last_discovery=0.0 이 재발견을 트리거하지 못한다 — stale 만 붙고 값이 안 돌아온다"
        );
        // 음성 대조 ①: 방금 발견한 휴리스틱 매핑은 재발견하지 않는다(매 틱 lsof 금지 —
        // 자원 거버넌스). 이 false 가 있어야 위 true 가 '항상 참' 이 아님이 증명된다.
        assert!(
            !needs_rediscovery(true, true, now, now - 1.0),
            "방금 발견한 매핑까지 매 틱 재발견하면 lsof 셸아웃이 폭주한다"
        );
        assert!(
            !needs_rediscovery(true, true, now, now - REDISCOVER_SECS),
            "경계값(정확히 임계)은 아직 재발견 아님"
        );
        assert!(
            needs_rediscovery(true, true, now, now - REDISCOVER_SECS - 0.1),
            "임계를 넘기면 재발견"
        );
        // 음성 대조 ②: 등록 매핑(heuristic=false)은 나이로 재발견하지 않는다 —
        // 신선도 가드의 범위(휴리스틱 한정)와 같은 경계다.
        assert!(
            !needs_rediscovery(true, false, now, 0.0),
            "등록 매핑을 나이로 갈아치우면 소유자 명시가 무의미해진다"
        );
        // 파일이 사라지면 매핑 종류와 무관하게 재발견한다.
        assert!(needs_rediscovery(false, false, now, now));
    }

    /// ★CEO 요구 증거(2026-09-04): 재발견이 타는 순서가 **결정론 우선**이다 —
    /// codex 는 lsof(열린 fd 직독)를 1순위로, 휴리스틱(날짜 디렉터리 최신 mtime)을
    /// 폴백으로만 쓴다. 이 순서가 뒤집히면 stale 을 유발한 바로 그 휴리스틱이 재발견에서도
    /// 1순위가 되어 같은 낡은 파일을 다시 집는다(가드가 무한 공회전).
    #[test]
    fn b6_rediscovery_prefers_deterministic_lsof_over_heuristic() {
        let src = include_str!("usage.rs");
        let body = src
            .split("fn discover_session_file(")
            .nth(1)
            .expect("discover_session_file 소실");
        let arm = body.find("\"codex\" =>").expect("codex 분기 소실");
        // 바이트 슬라이스로 자르지 않는다(멀티바이트 경계에서 패닉) — 시작만 잘라 상대 위치로 잰다.
        let tail = &body[arm..];
        let lsof = tail
            .find("discover_codex_rollout_lsof")
            .expect("결정론(lsof) 해소기 배선 소실");
        let heur = tail
            .find("or_else(|| discover_codex_rollout(")
            .expect("휴리스틱 폴백 배선 소실");
        assert!(
            lsof < heur,
            "휴리스틱이 lsof 보다 먼저 불린다 — 재발견이 낡은 파일을 다시 집는다"
        );
        assert!(heur < 300, "두 배선이 codex 분기 밖에서 잡혔다(위치 {heur}) — 핀이 헐겁다");
        // 결정론 해소기가 실제로 lsof 를 부르는지(이름만 그럴듯한 함수 아님).
        let resolver = src
            .split("fn discover_codex_rollout_lsof(")
            .nth(1)
            .expect("해소기 본문 소실");
        let call = resolver
            .find("Command::new(\"lsof\")")
            .expect("해소기가 lsof 를 부르지 않는다 — '결정론 경로' 라는 이름만 남는다");
        assert!(call < 300, "lsof 호출이 함수 본문 앞머리에 없다(위치 {call})");
    }

    /// ★CEO 요구 증거(2026-09-04 · 행위 검증): 결정론 해소기가 **실제로 열린 fd 를 읽어**
    /// rollout 경로를 돌려준다. 위 두 검체가 배선을 잡는다면 이것은 그 배선의 끝이 실제로
    /// 동작함을 잡는다 — 자기 프로세스가 연 파일을 자기 pid 로 되찾는다.
    ///
    /// macOS 한정인 이유: cargo test 레인이 macos-latest(.github/workflows/ci-branch.yml:29)
    /// 이고 `lsof` 는 그 플랫폼의 기본 바이너리(/usr/sbin/lsof)라 환경 때문에 조용히
    /// 무의미해지는 일이 없다. 다른 플랫폼에서는 위 배선 핀 둘이 계약을 지킨다.
    #[cfg(target_os = "macos")]
    #[test]
    fn b6_lsof_resolver_reads_open_rollout_fd() {
        use std::io::Write;
        let td = std::env::temp_dir().join(format!("cys-b6-lsof-{}", std::process::id()));
        let sessions = td.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let target = sessions.join("rollout-2026-09-04T04-00-00-abc.jsonl");
        let decoy = td.join("not-a-rollout.log");
        // 열어둔 채로 유지해야 lsof 가 본다(닫으면 fd 목록에서 사라진다).
        let mut f = std::fs::File::create(&target).unwrap();
        f.write_all(b"{}\n").unwrap();
        let mut d = std::fs::File::create(&decoy).unwrap();
        d.write_all(b"x\n").unwrap();

        // lsof 는 정규화된 경로를 낸다(macOS /var → /private/var 심링크) — 같은 기준으로 비교한다.
        let want = std::fs::canonicalize(&target).unwrap();
        let got = discover_codex_rollout_lsof(std::process::id());
        assert_eq!(
            got.as_deref(),
            Some(want.as_path()),
            "자기 pid 가 연 rollout fd 를 결정론으로 되찾지 못했다"
        );
        // 음성 대조: 패턴에 맞지 않는 열린 파일은 고르지 않는다(아무 fd 나 집는 것이 아님).
        let decoy_canon = std::fs::canonicalize(&decoy).unwrap();
        assert_ne!(got.as_deref(), Some(decoy_canon.as_path()));

        drop(f);
        drop(d);
        let _ = std::fs::remove_dir_all(&td);
    }
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

    // ───────── 0.14.42 RC2 — agy 관측 경로 재현 검체(수정 전 적색) ─────────

    /// RC2-a: lsof 는 선택 조건을 기본 **OR** 로 합친다. `-a` 가 없으면 "이 pid 의 파일 **또는** 기계 전체의
    /// LISTEN 소켓"이 나와 남의 포트(Discord·다른 부서의 agy)를 두드린다(2026-09-23 실측).
    #[test]
    fn agy_lsof_args_and_the_pid_with_the_listen_filter() {
        let a = super::agy_lsof_listen_args(61666);
        assert!(a.iter().any(|x| x == "-a"), "lsof 선택 조건이 OR 로 묶인다(-a 부재): {a:?}");
        let i = a.iter().position(|x| x == "-p").expect("-p");
        assert_eq!(a[i + 1], "61666");
        for need in ["-iTCP", "-sTCP:LISTEN", "-Fn"] {
            assert!(a.iter().any(|x| x == need), "{need} 부재: {a:?}");
        }
        // ★(R4-03) 두 호출 모두 비차단(-b)·경고 억제(-w) — 멈춘 네트워크 마운트에서 stat 대기 금지.
        for args in [super::agy_lsof_listen_args(61666), super::agy_lsof_files_args(61666)] {
            assert!(args.iter().any(|x| x == "-b") && args.iter().any(|x| x == "-w"), "비차단 인자 부재: {args:?}");
        }
    }

    /// ★(R4-03) agy lsof 는 시간 상한 안에서만 기다린다 — 상한 없는 `.output().await` 가 남으면 수집기 태스크가 영구 정지한다.
    #[test]
    fn agy_lsof_calls_are_time_bounded() {
        let src = include_str!("usage.rs");
        let body = &src[..src.find("#[cfg(test)]\nmod tests").expect("테스트 앵커")];
        assert_eq!(body.matches("Command::new(\"lsof\").args(args).kill_on_drop(true)").count(), 1);
        assert!(body.contains("tokio::time::timeout(AGY_LSOF_TIMEOUT, fut)"), "agy lsof 시간 상한 소실");
        assert_eq!(body.matches("agy_lsof_output(").count(), 3, "agy lsof 호출이 상한 래퍼를 우회한다");
    }

    /// RC2-a: agy 언어 서버 포트는 agy 가 연 로그의 첫머리에 결정론으로 적힌다(2026-09-23 실측 4개 로그 모두
    /// 273바이트 지점). HTTPS 줄만 — 바로 아래 HTTP 줄(포트+1)을 고르면 안 된다.
    #[test]
    fn agy_ls_https_port_is_read_from_the_log_line() {
        let head = "Log file created at: 2026/09/23 12:30:08\n\
            I0923 12:30:08.268390      25 server.go:625] Language server listening on random port at 65193 for HTTPS\n\
            I0923 12:30:08.268654      25 server.go:633] Language server listening on random port at 65194 for HTTP\n";
        assert_eq!(super::parse_agy_ls_https_port(head), Some(65193));
        // HTTP 줄만 있으면 없다(추측 금지) · 빈 입력·숫자 아님도 없다
        assert_eq!(
            super::parse_agy_ls_https_port("x] Language server listening on random port at 65194 for HTTP\n"),
            None
        );
        assert_eq!(super::parse_agy_ls_https_port(""), None);
        assert_eq!(
            super::parse_agy_ls_https_port("Language server listening on random port at 99999999 for HTTPS"),
            None
        );
        // 재기동으로 줄이 둘이면 마지막(현재) 포트
        let two = "Language server listening on random port at 1111 for HTTPS\n\
                   Language server listening on random port at 2222 for HTTPS\n";
        assert_eq!(super::parse_agy_ls_https_port(two), Some(2222));
    }

    /// RC2-a: `lsof -a -p <agy> -Fn` 출력에서 agy 자신의 로그 파일을 고른다(토큰 파일·심볼릭 cli.log 아님).
    #[test]
    fn agy_log_path_is_picked_from_lsof_names() {
        let out = "p61666\nfcwd\nn/Users/x\nf3\nn/Users/x/.gemini/antigravity-cli/antigravity-oauth-token\n\
                   f5\nn/Users/x/.gemini/antigravity-cli/log/cli-20260923_123008.log\nf6\nn127.0.0.1:65193\n";
        assert_eq!(
            super::agy_log_path_from_lsof(out),
            Some(PathBuf::from("/Users/x/.gemini/antigravity-cli/log/cli-20260923_123008.log"))
        );
        assert_eq!(super::agy_log_path_from_lsof("p1\nn/Users/x/.gemini/antigravity-cli/cli.log\n"), None);
        assert_eq!(super::agy_log_path_from_lsof(""), None);
    }

    /// RC2-b(정직 표기): 프로브 결과를 '경로 고장' 종류별로 분류한다 — 거부(HTTP 코드 보존)·쿼터 없음·도달 불가.
    #[test]
    fn agy_probe_outcomes_are_classified_not_swallowed() {
        let quota = r#"{"response":{"groups":[{"displayName":"Gemini Models","buckets":[{"window":"5h","remainingFraction":0.5}]}]}}"#;
        match super::classify_agy_probe(true, format!("{quota}\n200").as_bytes()) {
            super::AgyProbe::Ok(r) => assert_eq!(r[0].label, "5h"),
            other => panic!("정상 응답을 분류하지 못했다: {other:?}"),
        }
        // CSRF 가 아닌 거절은 코드를 보존한다(CSRF 거절은 아래 전용 검체 — 0.14.42 RC2-b 에서 분리)
        assert_eq!(
            super::classify_agy_probe(true, b"{\"code\":\"unauthenticated\",\"message\":\"token expired\"}\n401"),
            super::AgyProbe::Http(401)
        );
        assert_eq!(super::classify_agy_probe(true, b"{}\n200"), super::AgyProbe::NoQuota);
        assert_eq!(super::classify_agy_probe(false, b""), super::AgyProbe::Unreachable);
        assert_eq!(super::classify_agy_probe(true, b"\n000"), super::AgyProbe::Unreachable);
        assert_eq!(super::classify_agy_probe(true, b"garbage"), super::AgyProbe::Unreachable);
        // 오류 코드(계정 행 source_error) — 성공은 코드 없음
        assert_eq!(super::agy_error_code(&super::AgyProbe::Http(403)).as_deref(), Some("agy_http_403"));
        assert_eq!(super::agy_error_code(&super::AgyProbe::NoQuota).as_deref(), Some("agy_no_quota"));
        assert_eq!(super::agy_error_code(&super::AgyProbe::Unreachable).as_deref(), Some("agy_unreachable"));
        assert_eq!(super::agy_error_code(&super::AgyProbe::Ok(vec![])), None);
    }

    /// ★0.14.42 RC2-b(수정 전 적색): agy 1.2.9 언어 서버의 CSRF 거절(2026-09-23 22:11–22:15 오너 승인 라이브 프로브 실측
    /// 원문 그대로 — 헤더 없음 = `missing` · 틀린 값 = `invalid` · 평문 HTTP 포트도 같은 401)은 **전용 코드**다.
    /// 종전엔 `agy_http_401` 로만 보여 "무엇을 해야 값이 들어오나"(상태줄 연결)가 화면에서 드러나지 않았다.
    #[test]
    fn agy_csrf_rejection_is_its_own_code_and_outranks_the_rest() {
        for body in [
            &b"{\"code\":\"unauthenticated\",\"message\":\"missing CSRF token\"}\n401"[..],
            &b"{\"code\":\"unauthenticated\",\"message\":\"invalid CSRF token\"}\n401"[..],
        ] {
            assert_eq!(super::classify_agy_probe(true, body), super::AgyProbe::CsrfRequired, "{}", String::from_utf8_lossy(body));
        }
        assert_eq!(super::agy_error_code(&super::AgyProbe::CsrfRequired).as_deref(), Some("agy_csrf_required"));
        // 200 본문에 CSRF 라는 글자가 있어도 거절이 아니다(성공 판정은 코드가 한다)
        assert_eq!(super::classify_agy_probe(true, b"{\"note\":\"CSRF token\"}\n200"), super::AgyProbe::NoQuota);
        // 5xx 는 CSRF 거절이 아니다(서버 오류) — 코드 보존
        assert_eq!(super::classify_agy_probe(true, b"CSRF token store down\n503"), super::AgyProbe::Http(503));
        // 한 틱에 여러 실패가 섞이면 CSRF 가 가장 구체적인 사실이다(값을 얻는 길이 무엇인지 알려 주므로)
        for other in ["agy_http_401", "agy_no_quota", "agy_unreachable", "agy_no_port", "agy_no_process"] {
            assert!(
                super::agy_error_rank(super::AGY_ERR_CSRF) > super::agy_error_rank(other),
                "CSRF 가 {other} 보다 낮게 매겨졌다"
            );
        }
    }

    /// ★0.14.42 RC2-b(수정 전 적색): CSRF 거절은 결정론적이다 — 같은 agy pid 는 백오프 동안 다시 두드리지 않는다
    /// (종전: 좌석마다 15초마다 같은 401). 창이 지나거나 agy 가 재기동(새 pid)하면 다시 묻는다.
    #[test]
    fn agy_csrf_backoff_is_per_pid_and_expires() {
        let mut b: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
        let now = 1_000_000.0;
        assert!(!super::agy_csrf_backoff_active(&b, 61666, now), "기록 없음 = 묻는다");
        b.insert(61666, now + super::AGY_CSRF_BACKOFF_SECS);
        assert!(super::agy_csrf_backoff_active(&b, 61666, now + 15.0), "15초 뒤 같은 pid 를 다시 두드린다");
        assert!(super::agy_csrf_backoff_active(&b, 61666, now + super::AGY_CSRF_BACKOFF_SECS - 1.0));
        assert!(!super::agy_csrf_backoff_active(&b, 61666, now + super::AGY_CSRF_BACKOFF_SECS), "창이 지나면 다시 묻는다");
        assert!(!super::agy_csrf_backoff_active(&b, 70000, now + 15.0), "새 pid(재기동·업데이트)는 즉시 묻는다");
        assert!(super::AGY_CSRF_BACKOFF_SECS >= 600.0, "백오프가 폴링 주기 수준으로 짧아졌다");
    }

    /// ★fatal-fix R3-2: 창 라벨 → 길이 · 경보 근거 판정(순수 핀). 라벨은 좌석 보고가 실어 오는 임의 문자열일 수
    /// 있다 — 다바이트 라벨에서 문자 경계 panic 이 나면 워치독 틱이 죽는다(음성 대조).
    #[test]
    fn fatal_fix_rate_window_liveness_pins() {
        assert_eq!(window_secs("5h"), Some(18_000.0));
        assert_eq!(window_secs("7d"), Some(604_800.0));
        assert_eq!(window_secs("300m"), Some(18_000.0));
        for odd in ["?", "", "h", "0h", "가", "5시", "-5h", "5hh"] {
            assert_eq!(window_secs(odd), None, "{odd:?}");
        }
        let now = 2_000_000_000.0;
        let w = |label: &str, r: Option<f64>| RateWindow { label: label.into(), used_pct: 99.0, resets_at: r };
        assert!(rate_window_live(&w("5h", Some(now + 1.0)), now - 10.0, now));
        assert!(!rate_window_live(&w("5h", Some(now - 1.0)), now - 10.0, now), "리셋이 지났다");
        assert!(!rate_window_live(&w("5h", Some(now)), now - 10.0, now), "리셋 시각 = 지금");
        assert!(rate_window_live(&w("5h", None), now - 3600.0, now), "리셋 없음 · 창 안");
        assert!(!rate_window_live(&w("5h", None), now - 18_001.0, now), "리셋 없음 · 창 길이 초과");
        assert!(rate_window_live(&w("5h", Some(1200.0)), now - 60.0, now), "epoch 초가 아닌 리셋은 근거가 아니다");
        assert!(!rate_window_live(&w("5h", Some(1200.0)), now - 18_001.0, now), "그때는 나이로 판정");
        assert!(rate_window_live(&w("가", None), now - 1.0e9, now), "모르는 라벨은 남긴다(지우는 쪽 오판 금지)");
        assert!(rate_window_live(&w("5h", Some(f64::NAN)), now - 60.0, now));
        assert!(rate_window_live(&w("5h", None), 0.0, now), "관측 시각 모름 = 나이 판정 없음");
    }

    // ───────── fatal-fix (2026-09-24) — agy 수집기 틱(수정 전 적색) ─────────

    fn tick_daemon(tag: &str) -> Arc<Daemon> {
        let dir = std::env::temp_dir().join(format!("cys-agytick-{}-{}-{}", tag, std::process::id(), now_epoch() as u64));
        let _ = std::fs::create_dir_all(&dir);
        Daemon::new(dir.join("cysd.sock"))
    }

    fn add_agy_seat(d: &Arc<Daemon>) -> u64 {
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, Some("agy-tick".into()), 24, 80)
            .expect("create surface");
        *s.agent_meta.lock().unwrap() = Some(("gemini".into(), "agy".into()));
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.id
    }

    /// 다른 스레드가 `accounts` 락을 쥔 동안 수집기 한 틱을 **current_thread 런타임**에서 돌린다 — 끝났는가.
    fn tick_finishes_while_accounts_lock_is_held(d: &Arc<Daemon>) -> bool {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let holder = {
            let d = d.clone();
            std::thread::spawn(move || {
                let _g = d.accounts.lock().unwrap();
                let _ = held_tx.send(());
                let _ = release_rx.recv();
            })
        };
        held_rx.recv().unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        {
            let d = d.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async {
                    let mut ports = HashMap::new();
                    let mut backoff = HashMap::new();
                    agy_collector_tick(&d, &mut ports, &mut backoff, true).await;
                });
                let _ = done_tx.send(());
            });
        }
        let finished = done_rx.recv_timeout(Duration::from_secs(3)).is_ok();
        let _ = release_tx.send(());
        let _ = holder.join();
        finished
    }

    /// ★fatal-fix R4-F1: agy 수집기는 **모든 데몬에서** 15초마다 async 문맥에서 전역 `accounts` 표준 뮤텍스를 잡게
    /// 됐다(좌석 있음 = `agy_statusline_authoritative` · 없음 = `note_agy_error(None)`). 그 락이 막히면 수집기가 tokio
    /// 워커를 붙잡은 채 서고, 그 워커가 IO 드라이버를 돌리던 것이면 데몬의 **모든 소켓 요청**(ping·surface.list·GUI
    /// 입력·훅)이 멈췄다(워커 1개 런타임은 영구 정지 · r4 G 단계 A/B 재현). 이제 두 호출은 락을 **기다리지 않는다**
    /// (경합이면 그 틱을 건너뛴다 — 값은 다음 틱에 다시 적힌다).
    #[test]
    fn fatal_fix_agy_collector_tick_never_waits_on_the_accounts_lock() {
        let d = tick_daemon("no-seat");
        assert!(tick_finishes_while_accounts_lock_is_held(&d), "agy 좌석 없음: 수집기 틱이 accounts 락에서 멈췄다(런타임 정지)");
        let d = tick_daemon("seat");
        add_agy_seat(&d);
        assert!(tick_finishes_while_accounts_lock_is_held(&d), "agy 좌석 있음: 수집기 틱이 accounts 락에서 멈췄다(런타임 정지)");
    }

    /// ★fatal-fix W5: Windows 에는 agy 언어 서버 포트를 찾을 길(lsof·agy 로그의 포트 줄)이 없다 — RPC 경로가 구조적으로
    /// 불능인데 종전에는 영구 '관측 실패 · agy 포트 못 찾음/프로세스 없음'을 적어 값을 얻는 길(상태줄)을 가렸다.
    /// 이제 그 플랫폼에서는 프로브하지 않고 '상태줄 연결 필요' 코드를 적는다(상태줄 값이 들어오면 종전처럼 물러선다).
    #[test]
    fn fatal_fix_agy_collector_points_to_the_statusline_where_rpc_cannot_work() {
        let d = tick_daemon("win");
        add_agy_seat(&d);
        // 행 준비(부트 시드 대용 — 오류 코드는 이미 있는 행에만 싣는다)
        crate::accounts::note_rate(&d, "gemini", "", &[RateWindow { label: "5h".into(), used_pct: 1.0, resets_at: None }], "agy-rpc", now_epoch());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut ports = HashMap::new();
            let mut backoff = HashMap::new();
            agy_collector_tick(&d, &mut ports, &mut backoff, false).await;
        });
        let rows = crate::accounts::local_json(&d, now_epoch());
        let agy = rows.as_array().unwrap().iter().find(|r| r["provider"] == "antigravity").cloned().unwrap();
        assert_eq!(agy["source_error"], json!(AGY_ERR_STATUSLINE_REQUIRED), "{agy}");
    }
}

/// ★R3-1: /clear 재핀 판정·킬스위치·세션 id 형태의 순수 검체(프로덕션 무접촉 — 이음매 불요).
#[cfg(test)]
mod r3_1_verdict_tests {
    use super::{
        clear_repin_enabled_from, clear_repin_verdict, clear_transcript_fresh, is_plausible_session_id,
        lineage_proves_not_top_hook, CLEAR_FRESH_MAX_AGE, CLEAR_FRESH_MAX_BYTES,
    };
    use std::path::Path;

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const B: &str = "22222222-2222-4222-8222-222222222222";
    const N: &str = "44444444-4444-4444-8444-444444444444";

    fn p(stem: &str) -> String {
        format!("/Users/x/.claude/projects/-p/{stem}.jsonl")
    }

    /// 판정 전 분기 — (source, bound, agent, current, prev, transcript, held, depth) → 기대.
    /// 새 세션(ⓒ)은 참 · 훅 기원은 참으로 둔다(각각의 거부는 아래 전용 행이 본다).
    #[test]
    fn r3_1_verdict_table() {
        let (pa, pb, pn) = (p(A), p(B), p(N));
        #[allow(clippy::type_complexity)]
        let rows: Vec<(Option<&str>, bool, Option<&str>, Option<&str>, Option<&str>, &str, bool, Option<usize>, Result<&str, &str>)> = vec![
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), &pb, false, Some(1), Ok(B)),
            (Some("startup"), true, Some("claude"), Some(A), Some(&pa), &pb, false, Some(1), Err("not_clear")),
            (None, true, Some("claude"), Some(A), Some(&pa), &pb, false, Some(1), Err("not_clear")),
            (Some("clear"), true, Some("codex"), Some(A), Some(&pa), &pb, false, Some(1), Err("not_claude")),
            (Some("clear"), true, None, Some(A), Some(&pa), &pb, false, Some(1), Err("not_claude")),
            (Some("clear"), false, Some("claude"), Some(A), Some(&pa), &pb, false, Some(1), Err("caller_unbound")),
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), "/x/a;touch pwn.jsonl", false, Some(1), Err("bad_session_id")),
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), &pa, false, Some(1), Err("unchanged")),
            // ⓐ 연속성: 직전 등록 N(중첩 startup) ≠ 핀 A · 직전 등록 결측 ≠ 핀 A
            (Some("clear"), true, Some("claude"), Some(A), Some(&pn), &pb, false, Some(1), Err("discontinuous")),
            (Some("clear"), true, Some("claude"), Some(A), None, &pb, false, Some(1), Err("discontinuous")),
            // 핀 결측이면 연속성 비교 대상 없음 → 다음 조건으로
            (Some("clear"), true, Some("claude"), None, None, &pb, false, Some(1), Ok(B)),
            (Some("clear"), true, Some("claude"), None, Some(&pn), &pb, false, Some(1), Ok(B)),
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), &pb, true, Some(1), Err("held_by_other_seat")),
            // ⓑ 계통
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), &pb, false, Some(2), Err("nested_agent")),
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), &pb, false, Some(0), Err("lineage_unverified")),
            (Some("clear"), true, Some("claude"), Some(A), Some(&pa), &pb, false, None, Err("lineage_unverified")),
        ];
        for (i, (src, bound, agent, cur, prev, tr, held, depth, want)) in rows.into_iter().enumerate() {
            let got = clear_repin_verdict(src, bound, agent, cur, prev.map(Path::new), Path::new(tr), |_| held,
                || true, || depth.map(|d| (d, true)));
            assert_eq!(got.as_deref().map_err(|e| *e), want, "행 {i}");
        }
    }

    /// ★리뷰 F2: ⓒ 새 세션 · ⓑ 훅 기원 — 다른 관문을 모두 지난 등록에서 각각 단독으로 거부한다.
    #[test]
    fn r3_1_verdict_fresh_and_hook_origin() {
        let (pa, pb) = (p(A), p(B));
        let v = |fresh: bool, lin: Option<(usize, bool)>| {
            clear_repin_verdict(Some("clear"), true, Some("claude"), Some(A), Some(Path::new(&pa)), Path::new(&pb),
                |_| false, || fresh, || lin)
        };
        assert_eq!(v(true, Some((1, true))), Ok(B.to_string()));
        assert_eq!(v(false, Some((1, true))), Err("not_fresh_session"), "옛 대화(오래됨·큼)로 재핀했다");
        assert_eq!(v(true, Some((1, false))), Err("not_hook_origin"), "훅 밖(도구 셸) 직접 호출로 재핀했다");
        assert_eq!(v(true, Some((2, true))), Err("nested_agent"));
        assert_eq!(v(true, Some((0, true))), Err("lineage_unverified"));
        assert_eq!(v(true, None), Err("lineage_unverified"));
        // 새 세션 판정은 계통보다 먼저 — 옛 대화면 프로세스 표를 읽지 않는다.
        let called = std::cell::Cell::new(0u32);
        let _ = clear_repin_verdict(Some("clear"), true, Some("claude"), Some(A), Some(Path::new(&pa)), Path::new(&pb),
            |_| false, || false, || { called.set(called.get() + 1); Some((1, true)) });
        assert_eq!(called.get(), 0, "새 세션 거부 뒤에도 계통을 판독했다");
    }

    /// ★리뷰 F3: 연속성 기준을 옮기지 않을 등록 = 좌석 최상위 훅이 아니라고 **증명**된 것만.
    #[test]
    fn r3_1_not_top_hook_truth_table() {
        for (lin, want) in [
            (Some((1usize, true)), false),
            (Some((1, false)), true),
            (Some((2, true)), true),
            (Some((3, false)), true),
            (Some((0, false)), false),
            (Some((0, true)), false),
            (None, false),
        ] {
            assert_eq!(lineage_proves_not_top_hook(lin), want, "{lin:?}");
        }
    }

    /// ★리뷰 F2: 새 세션 증거 — 부재 · 방금 생긴 작은 파일만 참. 큰 파일 · 디렉터리 · 오래된 파일은 거짓.
    #[test]
    fn r3_1_transcript_fresh_table() {
        let dir = std::env::temp_dir().join(format!("cys-r31-fresh-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now();
        let absent = dir.join("33333333-3333-4333-8333-333333333333.jsonl");
        assert!(clear_transcript_fresh(&absent, now), "부재(훅 시점의 정상 모양)를 거부했다");
        let small = dir.join("small.jsonl");
        std::fs::write(&small, vec![b'x'; 2345]).unwrap();
        assert!(clear_transcript_fresh(&small, now), "방금 생긴 작은 파일(실측 2345 B)을 거부했다");
        let edge = dir.join("edge.jsonl");
        std::fs::write(&edge, vec![b'x'; CLEAR_FRESH_MAX_BYTES as usize]).unwrap();
        assert!(clear_transcript_fresh(&edge, now), "상한과 같은 크기를 거부했다");
        let big = dir.join("big.jsonl");
        std::fs::write(&big, vec![b'x'; CLEAR_FRESH_MAX_BYTES as usize + 1]).unwrap();
        assert!(!clear_transcript_fresh(&big, now), "큰 파일(옛 대화)을 새 세션으로 읽었다");
        let sub = dir.join("sub.jsonl");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(!clear_transcript_fresh(&sub, now), "디렉터리를 새 세션으로 읽었다");
        // 오래됨: 생성 시각을 읽을 수 있는 파일시스템에서만 판정한다(못 읽으면 크기만 — 그 갈래는 위 행들이 본다).
        if std::fs::metadata(&small).and_then(|m| m.created()).is_ok() {
            let later = now + CLEAR_FRESH_MAX_AGE + std::time::Duration::from_secs(5);
            assert!(!clear_transcript_fresh(&small, later), "생성 {}s 넘은 파일을 새 세션으로 읽었다",
                CLEAR_FRESH_MAX_AGE.as_secs());
            let earlier = now - std::time::Duration::from_secs(3600);
            assert!(clear_transcript_fresh(&small, earlier), "미래 생성 시각(시계 조정)을 '오래됨' 으로 읽었다");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 계통 판독(비쌈)은 싼 조건이 전부 통과한 뒤에만 불린다 — 거부 경로에서 프로세스 표를 읽지 않는다.
    #[test]
    fn r3_1_verdict_lineage_is_last_and_lazy() {
        let (pa, pb, pn) = (p(A), p(B), p(N));
        let called = std::cell::Cell::new(0u32);
        let bump = || {
            called.set(called.get() + 1);
            Some((1, true))
        };
        let _ = clear_repin_verdict(Some("clear"), true, Some("claude"), Some(A), Some(Path::new(&pn)),
            Path::new(&pb), |_| false, || true, bump);
        assert_eq!(called.get(), 0, "연속성 거부 뒤에도 계통을 판독했다");
        let _ = clear_repin_verdict(Some("clear"), true, Some("claude"), Some(A), Some(Path::new(&pa)),
            Path::new(&pb), |_| true, || true, bump);
        assert_eq!(called.get(), 0, "타 좌석 보유 거부 뒤에도 계통을 판독했다");
        let _ = clear_repin_verdict(Some("clear"), false, Some("claude"), Some(A), Some(Path::new(&pa)),
            Path::new(&pb), |_| false, || true, bump);
        assert_eq!(called.get(), 0, "좌석 결박 거부 뒤에도 계통을 판독했다");
        let _ = clear_repin_verdict(Some("clear"), true, Some("claude"), Some(A), Some(Path::new(&pa)),
            Path::new(&pb), |_| false, || true, bump);
        assert_eq!(called.get(), 1, "모든 싼 조건을 지났는데 계통 판독이 정확히 1회가 아니다");
    }

    #[test]
    fn r3_1_kill_switch_truth_table() {
        for (v, on) in [(None, true), (Some("1"), true), (Some(""), true), (Some("0"), false),
                        (Some(" 0"), true), (Some("00"), true), (Some("false"), true)] {
            assert_eq!(clear_repin_enabled_from(v), on, "{v:?}");
        }
    }

    #[test]
    fn r3_1_plausible_session_id_bounds() {
        assert!(!is_plausible_session_id(""));
        assert!(is_plausible_session_id(&"a".repeat(128)));
        assert!(!is_plausible_session_id(&"a".repeat(129)));
        assert!(is_plausible_session_id(A));
        assert!(is_plausible_session_id("x_y-Z9"));
        for bad in ["a b", "a;b", "a$b", "a`b", "a/b", "한글", "a\nb", "a.b"] {
            assert!(!is_plausible_session_id(bad), "{bad:?}");
        }
    }
}
