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
}
