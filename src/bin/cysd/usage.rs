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

// ───────────────── ★(0.14.42 · RV-R2NC-1 → RR3-R1-1 · G3ROLE-1) clear 직후 바닥 가드 — 재주입 고리 차단 ─────────────────
//
// 【왜】 918e7365 뒤 claude 좌석은 clear 사이클마다 합성 지침 **전문**(master 약 160KB · CEO 약 170~180KB — soul·메모리
// 색인·스킬 색인 포함 · worker 약 91KB)을 붙여 넣고, 이어 복원(SESSION_STATE·복원 주입 읽기)으로 몇 %p 더 자란다. 1M
// 창에서는 clear 뒤 바닥이 약 10~15% 라 무해하지만, 200K 창(문서 기본 `CYS_CLAUDE_CTX_WINDOW`)의 master·CEO 는 바닥이
// 이미 60~78% 다. 새 세션 파일이 래치를 재무장하므로(아래 `collect_for`) 바닥이 임계 이상이면 곧바로 `context.threshold`
// 가 다시 나고 CSO 는 방금 clear 한 좌석을 또 clear 해 같은 지침을 또 붙여 넣는다 — **유휴여도** 경보 쿨다운(5분)마다
// 도는 재주입 고리(①)이고 그동안 부서장은 일하지 못한다(ROLE).
//
// 【종전 수정의 한계】 6cda56b0·52b8c656 은 "교차 시점 바닥 + 고정 여유" 와 "창 유도 천장(200K 75)" 으로 임계를 올렸다.
// 바닥이 천장 − 5 이상이면(200K 71~) 올리지 않고 **바닥에서 곧바로 발화**했고(RR3-R1-1: 바닥 72 유휴 좌석 7사이클/40분),
// 판정을 교차 시점 바닥으로 해 복원 성장(+8%p)이 정착 창 안에서 올린 임계를 넘으면 작업 0 으로 발화했다(G3ROLE-1). 둘 다
// 바닥 모형(지침 크기·복원 성장량·창)을 추정해 여유를 정한 탓이다.
//
// 【무엇 — 구조적 불변식】 바닥을 추정하지 않고 **잰다**. 좌석마다 "마지막 발화 → 그 뒤 첫 세션 교체(clear) → 정착 창 안
// 최고치 = 이 세션의 바닥" 을 기록하고, 그 세션의 발화 임계를 **바닥 + 여유**(히스테리시스 — [`ctx_floor_bar`])로 둔다.
// 정착 창 = 시작점 + [`CTX_FLOOR_SETTLE_SECS`](시작점은 사이클의 quiescing 이 이어지는 동안 뒤로 밀린다), 그 뒤에도 바닥이
// [`CTX_FLOOR_SETTLE_QUIET_SECS`] 안에 올랐으면(복원이 아직 도는 중) 최대 [`CTX_FLOOR_SETTLE_MAX_SECS`] 까지. 그 전에라도
// **좌석 출력이 조용해지면**(clear 사이클 자신의 입력 = 붙여넣기 → 복원 턴이 끝남 · 시작점 + [`CTX_FLOOR_SETTLE_MIN_SECS`]
// 이후 · [`CtxLoopGuard::note_idle`]) 닫힌다 — 그 뒤 들어온 작업(대기열 배달·push 는 조용함 3초 뒤라 언제나 틈이 있다)은
// 바닥이 아니라 성장이다. 시간 창만으로 재면 바쁜 좌석의 바닥이 복원 뒤 몇 분의 작업만큼 부풀어(200K master 72 → 77~85)
// 여유 있는 좌석이 차단기(Probe·Stopped — 바닥+10 = 선제 압축점 위)로 오판된다(② — 샌드박스 94.9%).
//   (I1) clear 뒤 세션의 발화는 **잰 바닥 위로 실제로 자란 뒤**다 — 정상·Limited 는 [`CTX_FLOOR_MIN_ROOM`] 이상, Stopped 는
//        [`CTX_FLOOR_MIN_GROWTH`] 이상, Probe 는 1%p 이상. 정착 창 안에서는 바닥이 관측을 따라 오르므로 발화가 없다(아래
//        뒷문 하나만 예외). 따라서 유휴 좌석(바닥에서 자라지 않음)의 clear 직후 재발화는 창·지침 크기·복원 성장량과
//        무관하게 **0** 이고, 사이클 수는 실제 성장량이 부른다(시간·경보 쿨다운이 아니라).
//   (I2) 바닥 위로 [`CTX_FLOOR_ROOM`] 이상 실제로 자란 컨텍스트는 **반드시** 발화한다(② 무clear 금지). 올린 임계는 가능한
//        한 Claude Code 한계 아래다 — 선제 압축 천장([`ctx_floor_ceiling`] · 200K 75)이 우선, 그 안에 최소 여유가 없으면
//        차단 상한([`ctx_floor_hard_cap`] · 200K 80 — 사이클이 차단점 전에 끝난다), 그 안에도 없으면 clear 가 여유를 만들 수
//        없는 좌석(Probe·Stopped)이다.
//   (I3) 연속 clear 차단기: 차단 상한 근처 바닥은 **1회만** 다시 clear 해 보고(Probe — max(차단 상한, 바닥+1)), 다음 clear
//        뒤에도 그러면 자동 clear 를 멈춘다(Stopped — 바닥 + [`CTX_FLOOR_MIN_GROWTH`] 까지 실제로 자랄 때만 재무장 · 그 전에는
//        Claude Code 자체 자동 압축이 받는다) + 오너 경보(1·2·4·8…번째 — 무한 재시도·무음 둘 다 금지). 바쁜 좌석도
//        같다 — 차단 상한 근처 바닥에서 바닥+1%p 마다 사이클을 도는 것은 작업 1%p 에 지침 전문 재주입이라 고리와 같다.
//   (I4) 정착 창 뒷문(② 봉인): 창 안에서도 max(기본, 차단 상한)에서는 발화한다 — clear 직후 몇 분 만에 실제 작업으로 차단
//        상한을 넘는 좌석이 창이 닫힐 때까지 끌려가지 않게. 뒷문은 한 번 쓰면 바닥이 차단 상한에서 먼 세션(정상·Limited)의
//        창 밖 발화 뒤에야 다시 무장한다 — 붙여넣기·복원만으로 차단 상한을 넘는 유휴 좌석은 뒷문 1회 뒤 Stopped 로 멈추고,
//        차단 상한 근처의 바쁜 좌석도 뒷문 1회 뒤에는 Stopped(바닥+10) 사이클만 돈다.
//   창 미상은 200K 로 본다(천장·상한이 낮아지는 쪽). 창이 바뀌면(1M 전환) 잰 바닥을 토큰 비율로 옮긴다.
//   (I5) ★(ROLE-R4-1 · R2NC5-1) 차단기(I3)의 재료는 **복원 턴 끝의 바닥**이다([`ctx_floor_bar_measured`]). 사이클은
//        clear·재주입 동안 대기열·채널·스케줄 배달을 붙잡고(`MachineHold::Quiescing`) 붙여넣기 직후 풀기 때문에, 짧은 복원
//        턴(라이브 master 14~53초) 뒤 붙잡혔던 배달이 조용함 3초마다 몰려 와 최소 창 60초 안에 처리된다 — 그 몫이 정착 창
//        최고치에 들어 참 바닥 72 인 200K master 가 76~78 로 재여 Probe → Stopped(바닥+10 · 선제 압축점 위)로 가고 cys clear
//        가 영영 나지 않았다. 복원 턴 끝은 둘 중 먼저 성립하는 것에서 확정한다: 복원 턴의 보고(시작점 뒤 상승) 뒤 **첫 대기열
//        배달**(대기열은 턴이 끝난 좌석에만 인계한다 — claude 프롬프트 경계 배달은 조용함을 기다리지 않아 2초 틈이 없을 수
//        있다 · 인계 뒤 그 배달이 제출(CR)되기 전 1초 안의 늦은 복원 끝 보고는 싣는다), 또는 그 뒤 처음 온 ≥2초 조용함 **다음의
//        좌석 입력**(writer Inject 끝 · 직접 send · 사람 입력 — [`CtxIdleObs`]). 입력 없이 출력이 다시 흐른 틈은 같은 턴이다 —
//        확정하지 않는다. 확정된 복원 끝에 여유가 있으면 정착 창 최고치가 차단기 높이여도 Limited(확인 없음 ·
//        보고 바닥 = 복원 끝) · 막대는 정착 창 최고치의 미확인 막대(max(차단 상한, 최고치+1)) 그대로 — 어떤 막대도 앞당기지
//        않는다. 확정 뒤 정착 창은 사이클이 붙잡았던 **대기열 항목**(시작점 = quiescing 해제 전에 들어온 것 ·
//        [`CtxIdleObs::queue_oldest`])이 남아 있는 동안 몰림 조용함([`CTX_FLOOR_BURST_QUIET_SECS`] · 대기열 막힘)에서만 닫혀
//        붙잡혔던 배달 전체가 최고치에 든다 — 배달만으로는 발화하지 않는다(몰림 길이·크기와 무관하게 사이클마다 도는 고리
//        없음). ★(R2NC6-1 · RV-ROLE-CF42-1) 그 항목이 다 배달되면 마지막 배달 뒤 ≥2초 조용함에서 닫는다 — 시작점 뒤에 들어온
//        배달은 새 작업이다(d9638152 는 확정 뒤 언제나 15초를 기다려, 턴 사이 틈이 2~15초뿐인 바쁜 좌석의 창이 300~600초
//        열린 채 작업을 최고치에 실었다 — 막대 82~86 · 둘째 세션부터 Claude 선제 압축이 cys clear 를 가로챘다). 뒷문 재무장도
//        복원 끝에 여유가 있고 막대가 차단 상한 이하인 창 밖 발화에서 한다(정착 창 최고치가 76~79 여도).
//   (I6) ★(R2NC5-1 (b)) Claude 자체 압축(같은 세션 파일 · 컨텍스트가 잰 바닥보다 [`CTX_FLOOR_COMPACT_DROP`] 이상 낮음)을 본
//        차단기 영역(또는 막대 > 차단 상한) 세션은 재무장한다 — Stopped 는 새 세션이 없으면 다시 재지 않아 오너가 손으로 clear
//        할 때까지 cys 사이클(저장·지침 재주입)이 영영 없었다. ★(R1-RUNAWAY-1) 재무장은 기본 임계로 곧장 가지 않고 **압축 뒤
//        바닥을 다시 잰다**(압축 관측 시각을 시작점으로 정착 창을 새로 연다 · 뒷문 없음) — 압축 뒤 SessionStart:compact 훅이
//        지침 전문을 다시 읽게 해 압축 뒤 수준이 기본 임계 위(60~70%)일 수 있고, d9638152 는 그 수준에서 성장 0 으로 곧바로
//        발화해 붙여넣기+복원이 선제 압축점 위인 좌석이 붙여넣기 → 압축 → 재읽기 → 발화를 주기 신호마다 돌았다(①). 이제
//        압축 뒤 발화는 그 바닥 위로 실제로 자란 뒤(바닥 + 여유 · 천장·차단 상한)이고 그 clear 가 바닥을 다시 잰다(참으로 가득
//        찬 좌석은 다시 Stopped — 압축 1회당 사이클 ≤ 1, 실제 성장이 부른다).
//        ★(RV2-ROLE-1) 압축 뒤 바닥은 **재읽기 몫**만 잰다 — 압축 관측부터 [`CTX_FLOOR_REREAD_SECS`](90초 · 연장·조용함·몰림 규칙
//        없음). 압축은 작업 턴 도중에 와 그 턴이 몇 분 이어질 수 있다(종전 clear 뒤 창 규칙은 480초 턴의 작업을 '압축 뒤 바닥'에
//        실어 63~65 를 80 으로 쟀다). 그 창 안의 두 번째 압축도 다시 잰다 · clear 뒤 창 안의 압축(차단기 세션)도 창이 끝나길
//        기다리지 않고 곧바로 재무장한다.
//        ★(ADV2-R1-1) 압축 뒤 발화는 **clear 로 돌아오는 바닥**(압축 전 clear 세션의 복원 끝 + clear 마다 되풀이되는 몰림) + 1
//        이상에서만 — 그보다 낮은 cys clear 는 컨텍스트를 오히려 올리고 그 붙여넣기가 다음 압축을 불러 성장 약 4.5%p 마다
//        clear → 압축 → 재무장을 돌았다. 그 바닥이 선제 압축점 높이면(사이클 자신의 입력이 clear 뒤 창 안에서 압축을 불렀다)
//        압축 뒤 cys 사이클을 걸지 않는다(Claude 압축이 받는다).
//   (I7) ★(RV2NC-E20-1 · ①) 복원 끝엔 여유가 있지만 정착 창 최고치(사이클이 붙잡은 몰림 · 복원 턴 동안 선 회신 몰림 포함)가
//        차단 상한 근처인 **얇은 세션**은 확인을 영구히 끄지 않는다 — 미확인 막대 max(차단 상한, 최고치 + 2)(실제 성장 ≥ 1%p 를
//        입증하는 최소 막대)에서 발화하고, 그 발화를 **성장 속도**로 가른다([`CTX_FLOOR_THIN_SECS`] — 반올림 뺀 실제 성장 하한이
//        분당 0.2%p 이상이면 일하는 좌석). 느리면(주기 신호·유휴) 다음 세션은 몰림을 바닥으로 확정해(Backlog) 그 위 실제 성장
//        5%p 마다만 clear 한다 — 사이클 수 ≤ 2 + 성장/5(e20f1cb6 은 드릴 1시간 10~11회). 빠르면(분당 0.5%p 이상 일하는 좌석) 종전처럼
//        차단 상한 근처에서 clear(선제 압축 전 · ②). 이것이 ①·② 사이의 명시적 빈도 상한이다. 대기열 몰림의 기준도 복원 끝을
//        확정한 입력 시각까지 넓힌다(복원 턴 동안 선 항목 = 사이클의 몰림).
//   (I8) ★(RV2NC-E20-2 · ②) 채널 inbox 행·스케줄 직접 push(바쁨을 보지 않는 기계 주입)는 clear 사이클의 **복원 턴 동안** 보류·큐
//        우회한다([`CtxLoopGuard::restore_turn_open`] · 상한 [`CTX_FLOOR_RESTORE_HOLD_SECS`]) — 복원 턴 도중에 타이핑된 행은 Claude 가
//        복원 턴 뒤 조용함 없이 곧바로 처리해 복원 끝 바닥을 오염시켰다(여유 있는 master 76 → Probe → Stopped). 그래도 복원 턴
//        도중에 대기열 아닌 Inject 가 끝나면(다른 생산자) 복원 끝을 그 순간의 최고치(하한)로 확정하고 오염으로 적는다 — 차단기는
//        그 하한이 차단 상한 근처일 때만.
//
// 【실패 방향】 재료가 없으면(세션 교체 미관측 · 정착 창 관측 없음 · 발화 이력 없음 = 부트·phoenix --resume 첫 교차)
// **종전대로 기본 임계에서 발화**한다 — 사이클 1회(데몬 세대당)이지 무clear 가 아니다. clear 로 여유를 못 만드는 좌석
// (Stopped)의 처방(1M · 지침 축소)은 오너 결정으로 남는다 — 자동 압축을 끈 좌석은 차단점에서 멈출 수 있음을 경보가 말한다.
// 남은 한계(수치로 묶임): 복원 성장이 정착 창 최대 길이(붙여넣기 뒤 약 10분) 밖으로 이어지면 그만큼은 작업으로 센다 —
// 그 좌석의 사이클은 복원 시간에 묶인다(경보 쿨다운 5분이 아니라 · 실측 복원은 300초 안). 복원 턴 끝이 확정되지 않는
// 좌석(복원 뒤 입력 없음 · 출력이 한 번도 2초 이상 끊이지 않음 · 입력 기록 없는 `send-key` 단독 제출)은 종전대로 정착 창
// 최고치로 판정한다 — 복원 턴 **안에서** 계속 일하거나(워커 RESUME 긴 턴) 복원 턴 중에 Claude 내부 대기열에 선 입력(채널
// 행·스케줄 push 는 바쁨을 보지 않는다)이 곧바로 이어지면 그 몫은 바닥에 든다 — 그 좌석이 Stopped 로 가도 Claude 압축
// 뒤 재무장(I6)이 사이클을 되돌린다. 차단 상한 근처 **복원 끝** 바닥(200K 에서 붙여넣기+복원 ≥ 76%)의 좌석은 1회 재시도
// 뒤 자동 clear 를 멈추고(Stopped) 선제 압축점(83.5%)은 Claude 자동 압축이 받는다(압축마다 재무장 1회) — 처방(1M · 지침
// 축소)은 오너 결정.
//
// 【천장 — ★R2NC3-1】 천장은 창 크기와 Claude Code 자체 한계에서 유도한다. Claude Code 2.1.282(설치본 strings 실측 ·
// 실행 안 함): 유효 창 = 창 − min(최대 출력, 20000) · **선제 압축점** = 유효 창 − 13000(압축 창 source 가 `auto` 가
// 아닐 때 — env `CLAUDE_CODE_AUTO_COMPACT_WINDOW`·settings·서버 clientdata/experiment, 그리고 opus-4-6/4-8/5/5-5·
// sonnet-4-6 의 200K 창은 `model-default` 라 **기본값이 이쪽**) · **차단점** = 유효 창 − 3000(자동 압축 끔
// `autoCompactEnabled=false`·`DISABLE_AUTO_COMPACT`·`DISABLE_COMPACT`, 또는 source≠auto — 프롬프트를 보내지 않는다).
// 200K 창에서 83.5%·88.5% 다. 선제 압축 천장 = 창 − (20000 + 13000 + 사이클 여유 15000)(85 캡) — 200K 75% · 1M 85%.
// 차단 상한 = 창 − (20000 + 3000 + 사이클 여유 15000) — 200K 80% · 1M 95%(그 퍼센트에서 발화해도 사이클이 차단점 전에
// 끝난다 · 선제 압축이 먼저 올 수는 있다 — 그래도 교차는 압축점 전이라 cys clear 는 난다). 사용자가 압축 창을 모델 창보다
// 작게 준 좌석(`CLAUDE_CODE_AUTO_COMPACT_WINDOW`·`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`)은 데몬이 보지 못한다(USER-MANUAL 고지).

/// clear(세션 교체) 뒤 바닥을 재는 정착 창(초) — 재주입 대기(≤75s)·붙여넣기 처리·복원 읽기를 덮는다(실측 300초 안).
pub const CTX_FLOOR_SETTLE_SECS: f64 = 300.0;
/// 정착 창 연장 — 기본 창이 끝나도 바닥이 이 시간(초) 안에 올랐으면(복원이 아직 도는 중) 창을 연다.
pub const CTX_FLOOR_SETTLE_QUIET_SECS: f64 = 120.0;
/// 정착 창의 최대 길이(시작점부터 · 초) — 연장이 끝없이 이어져 발화가 막히지 않게(②). 실측 복원(≤300s)의 2배.
pub const CTX_FLOOR_SETTLE_MAX_SECS: f64 = 600.0;
/// 정착 창의 최소 길이(시작점부터 · 초) — 이보다 이른 조용함은 창을 닫지 않는다(붙여넣기 처리 시작 전의 틈 · 여러 입력으로
/// 나뉜 복원). 좌석이 복원 턴을 이보다 일찍 끝내고 곧바로 일을 받으면 그 몫(이 시간 + 한 턴)만 바닥에 든다.
pub const CTX_FLOOR_SETTLE_MIN_SECS: f64 = 60.0;
/// 좌석 출력이 이만큼(초) 조용하면 턴이 끝난 것이다(작업 중 claude·codex TUI 는 스피너·경과 시간을 1초보다 잦게 그린다).
/// 대기열 배달은 조용함 3초(`CYS_QUEUE_QUIET_SECS`) 뒤라 복원 턴과 다음 작업 사이에는 언제나 이만한 틈이 있다.
pub const CTX_FLOOR_IDLE_QUIET_SECS: f64 = 2.0;
/// ★(ROLE-R4-1 · R2NC5-1) 복원 끝 바닥이 확정된 뒤, 사이클이 붙잡았던 대기열 항목이 **아직 남아 있는 동안** 정착 창을 닫는
/// 조용함(초) — 대기열 배달 사이 간격은 조용함 3초 · 최소 간격 10초(`CYS_QUEUE_MIN_INTERVAL_SECS` · 머리 대기가 길면 줄어든다)
/// 안이라 몰림 속 틈은 이보다 짧다. 몰림 **전체**가 정착 창 최고치에 들어야 막대(≥ 최고치 + 1)가 몰림만으로 발화하지 않는다
/// (몰림이 복원 끝 바닥의 여유보다 커도 사이클마다 도는 고리가 없다 · ①). 남은 항목이 이만큼 조용한데도 배달되지 않으면(대기열
/// 막힘) 닫는다. ★(R2NC6-1 · RV-ROLE-CF42-1) 붙잡혔던 항목이 다 배달된 뒤에는 이 규칙이 아니다 — 마지막 배달 뒤 ≥
/// [`CTX_FLOOR_IDLE_QUIET_SECS`] 조용함에서 닫는다([`CtxLoopGuard::note_idle`] · 종전 d9638152 는 확정 뒤 언제나 15초를 요구해
/// 대기열이 선 바쁜 좌석(턴 사이 틈 2~15초 — 최소 간격 10초 · 프롬프트 경계 배달은 조용함을 기다리지 않는다)의 창이 시작점 +
/// 300~600초까지 열린 채 작업을 최고치에 실었다 — 막대 = 최고치 + 1 이 82~86 으로 밀려 Claude 선제 압축이 cys clear 보다 먼저 왔다 · ②).
pub const CTX_FLOOR_BURST_QUIET_SECS: f64 = 15.0;
/// ★(ROLE-R4-1 · R2NC5-1) 대기열 배달(인계)로 복원 끝 바닥을 확정한 뒤, 그 배달의 **제출(CR)이 쓰이기 전**이고 이 시간(초)
/// 안인 보고는 아직 복원 턴의 늦은 보고다 — 턴이 끝나고 상태줄 보고(디바운스 + 보고 명령)가 도착하기 전에 대기열이 프롬프트
/// 경계에서 곧바로 인계할 수 있다. 제출(writer Inject 의 CR · `InjectTrack::done_at`) 뒤의 보고는 새 메시지를 담을 수 있어
/// 싣지 않는다(샌드박스 반례: 인계 0.7초 뒤 보고가 22KB 배달을 담아 참 바닥 72 좌석이 77 로 재여 Stopped).
pub const CTX_FLOOR_INPUT_GRACE_SECS: f64 = 1.0;
/// ★(RV2NC-E20-1 · ①) **얇은 세션**(복원 끝엔 여유가 있지만 복원 뒤 배달·회신까지 담은 정착 창 최고치가 차단 상한 근처 —
/// 최고치 + [`CTX_FLOOR_MIN_ROOM`] > 차단 상한)의 발화가 **일하는 좌석**의 것인가를 가르는 시간(초). 정착 창이 닫힌 뒤 발화까지
/// **표시 성장 − 1**(1%p 반올림을 뺀 실제 성장의 하한)이 이 시간에 최소 여유(5%p)를 채우는 속도(분당 0.2%p) 이상이면 일하는
/// 좌석이다. ★명시적 빈도 상한(①·② 사이의 선택): 그보다 느린 좌석(주기 신호·유휴 — 분당 0.03~0.15%p)의 얇은 사이클은 연속
/// 두 세션에 걸쳐 주지 않는다 — 둘째 세션부터는 몰림을 바닥으로 확정해([`CtxFloorRegime::Backlog`]) 그 위 최소 여유 성장마다
/// 한 번이다(사이클 수 ≤ 2 + 성장/5 · 종전 e20f1cb6 은 사이클마다 몰림이 최고치를 다시 채우는 유휴 좌석을 주기 신호 1~4%p
/// 마다 돌렸다). 분당 0.5%p 이상 일하는 좌석은 종전대로 차단 상한 근처에서 clear 한다(Claude 선제 압축 전 · ②).
pub const CTX_FLOOR_THIN_SECS: f64 = 1500.0;
/// ★(RV2-ROLE-1) Claude 압축 뒤 바닥을 다시 재는 창(초 · 압축 관측 시각부터 · 연장 없음) — 압축 뒤 SessionStart:compact 훅이
/// 시키는 지침 재읽기(모형 20초 · 100KB 지침 Read 몇 번)를 덮는다. 압축은 작업 턴 **도중**에 오고 그 턴은 몇 분 더 이어질 수
/// 있어, 조용함(턴 끝)이나 clear 뒤 정착 창(최소 60초 · 최대 600초)을 기다리면 턴의 남은 작업이 '압축 뒤 바닥'에 들었다
/// (재검증자 반례: 480초 턴 · 참 압축 뒤 수준 63~65 가 80 으로 재여 Stopped · 압축 두세 번에 사이클 한 번). 이 창 안에서는
/// 발화가 없다(막대 ≥ 최고치 + 1) — 창이 짧아 뒷문이 필요 없다(분당 3%p 에서도 4.5%p).
pub const CTX_FLOOR_REREAD_SECS: f64 = 90.0;
/// ★(RV2NC-E20-2) 대기열 배달의 인계(`last_queue_delivery_at`) 뒤 그 배달의 제출(writer Inject 끝)까지로 보는 시간(초) — 이
/// 안의 Inject 끝은 대기열 배달의 것이다(복원 턴 오염 재료가 아니다 · 붙여넣기 500ms + 큰 본문 쓰기).
pub const CTX_FLOOR_QUEUE_CR_SECS: f64 = 5.0;
/// ★(RV2NC-E20-2) clear 사이클의 복원 턴 동안 바쁨을 보지 않는 기계 주입(채널 inbox 행 · 스케줄 직접 push)을 보류하는 상한(초 ·
/// 정착 창 시작점부터) — 라이브 복원 턴 14~53초의 두 배 남짓. 복원 턴 끝(≥2초 조용함)을 보면 곧바로 푼다 · 조용함이 오지 않는
/// 좌석(끊김 없는 긴 턴)도 이 시간 뒤에는 종전대로 주입한다(③ 주기 신호·원격 steer 무기한 보류 없음).
pub const CTX_FLOOR_RESTORE_HOLD_SECS: f64 = 120.0;
/// 연속 clear 차단기(Stopped)가 자동 clear 를 다시 무장하는 실제 성장(%p) — 잰 바닥 + 이 값.
pub const CTX_FLOOR_MIN_GROWTH: u8 = 10;
/// ★(R2NC5-1 (b)) 같은 세션에서 컨텍스트가 잰 바닥보다 이만큼(%p) 이상 낮게 보고되면 Claude 자체 압축(자동·수동 /compact)
/// 으로 본다 — 한 세션 안의 컨텍스트는 압축 말고는 줄지 않는다(실측 압축 83.5 → 20~35%).
pub const CTX_FLOOR_COMPACT_DROP: u8 = 10;
/// 바닥이 임계를 막은 좌석의 다음 clear 여유(%p) — 올린 임계 = 바닥 + ROOM(천장·차단 상한까지).
pub const CTX_FLOOR_ROOM: u8 = 15;
/// 올린 임계가 잰 바닥 위로 최소한 줘야 하는 여유(%p). 천장·차단 상한 때문에 이보다 좁으면 그 영역에서 올리지 않는다 —
/// 바닥 1~4%p 위 임계는 주기 신호(heartbeat·각성 핑) 몇 개마다 사이클이 도는 고리와 같다(★R2NC3-1 · RR3-R1-1).
pub const CTX_FLOOR_MIN_ROOM: u8 = 5;
/// 선제 압축 천장의 절대 캡(%) — 큰 창(1M)에서도 이 위로는 Raise 하지 않는다. 실제 천장은 [`ctx_floor_ceiling`].
pub const CTX_FLOOR_CEIL: u8 = 85;
/// "발화하지 않음" 임계 — 퍼센트(≤100)가 닿을 수 없는 값. Stopped 의 바닥 + 성장이 100 을 넘을 때.
pub const CTX_FLOOR_NEVER: u8 = 101;
/// Claude Code 요약 출력 예약 상한 — 유효 창 = 창 − min(최대 출력, 이 값). 상한을 쓴다(보수 — 실제 예약은 이하).
pub const CC_SUMMARY_RESERVE_TOKENS: u64 = 20_000;
/// Claude Code 자동 압축 버퍼 — 선제 압축점 = 유효 창 − 이 값.
pub const CC_AUTOCOMPACT_BUFFER_TOKENS: u64 = 13_000;
/// Claude Code 차단 버퍼 — 차단점 = 유효 창 − 이 값(자동 압축 끔·비-auto 압축 창).
pub const CC_BLOCKING_BUFFER_TOKENS: u64 = 3_000;
/// 올린 임계에서 발화한 뒤 CSO 사이클의 저장 지시가 처리될 때까지 좌석이 더 쓰는 양의 여유(토큰).
pub const CTX_FLOOR_CYCLE_MARGIN_TOKENS: u64 = 15_000;
/// 창을 모를 때 가정하는 창 — claude 최소 창(200K). 작은 창을 가정해야 천장이 낮아져 발화 쪽으로 실패한다.
pub const CTX_FLOOR_ASSUMED_WINDOW: u64 = 200_000;

/// (창 − 예약)으로 **보이는** 가장 큰 정수 퍼센트 — 상태줄 `used_percentage` 는 반올림이라 c% 는 (c+0.5)% 직전까지다.
/// c ≤ (200·(창 − 예약) − 창) / (2·창). 창 미상(None·0)은 [`CTX_FLOOR_ASSUMED_WINDOW`]. 창이 예약보다 작으면 0.
fn ctx_pct_below_reserve(window: Option<u64>, reserve: u64) -> u8 {
    let w = window.filter(|w| *w > 0).unwrap_or(CTX_FLOOR_ASSUMED_WINDOW);
    let usable = w.saturating_sub(reserve);
    let pct = usable.saturating_mul(200).saturating_sub(w) / w.saturating_mul(2);
    pct.min(100) as u8
}

/// ★(R2NC3-1) 선제 압축 천장(%) — 순수 · 핀 `ctx_floor_ceiling_*`. (천장 퍼센트로 보이는 최대 토큰) + 사이클 여유 ≤
/// Claude Code 선제 압축점(< 차단점)이 되는 가장 큰 정수 퍼센트, [`CTX_FLOOR_CEIL`] 캡. 200K 75 · 1M 85 · 미상 = 200K.
pub fn ctx_floor_ceiling(window: Option<u64>) -> u8 {
    let reserve = CC_SUMMARY_RESERVE_TOKENS + CC_AUTOCOMPACT_BUFFER_TOKENS + CTX_FLOOR_CYCLE_MARGIN_TOKENS;
    ctx_pct_below_reserve(window, reserve).min(CTX_FLOOR_CEIL)
}

/// ★(RR3-R1-1) 차단 상한(%) — 순수 · 핀 `ctx_floor_hard_cap_*`. (상한 퍼센트로 보이는 최대 토큰) + 사이클 여유 <
/// Claude Code 차단점이 되는 가장 큰 정수 퍼센트 — 이 위에서 발화하면 자동 압축을 끈 좌석은 저장 지시가 차단점에 막혀
/// 사이클이 끝나지 못한다. 200K 80 · 1M 95 · 미상 = 200K. 언제나 [`ctx_floor_ceiling`] 이상.
pub fn ctx_floor_hard_cap(window: Option<u64>) -> u8 {
    // +1 토큰 — 차단은 '도달'에서 일어나므로 상한으로 보이는 최대 토큰 + 여유가 차단점에 **미치지 않아야** 한다(엄격).
    let reserve = CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS + CTX_FLOOR_CYCLE_MARGIN_TOKENS + 1;
    ctx_pct_below_reserve(window, reserve).max(ctx_floor_ceiling(window))
}

/// clear 뒤 세션의 바닥이 정한 발화 영역(순서 = 심각도).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CtxFloorRegime {
    /// 바닥 + 최소 여유가 선제 압축 천장 안 — 발화 = min(바닥 + ROOM, 천장).
    Raise,
    /// 천장 밖 · 차단 상한 안 — 발화 = min(바닥 + ROOM, 차단 상한). 선제 압축이 사이클보다 먼저 올 수 있다.
    Limited,
    /// ★(RV2NC-E20-1) 복원 끝에는 여유가 있지만 복원 뒤 배달·회신까지 담은 정착 창 최고치가 차단 상한 근처이고, 직전 clear
    /// 세션도 차단 상한 근처였다(얇은 세션의 느린 발화 · 차단기 영역 — 확인) — clear 마다 되풀이되는 그 몰림을 바닥으로 보고
    /// 최고치 위로 실제로 [`CTX_FLOOR_MIN_ROOM`] 자랄 때만(표시 최고치 + 6) 발화. 일하는 좌석의 빠른 발화가 확인을 푼다.
    Backlog,
    /// 차단 상한 안에도 최소 여유가 없다 · 아직 확인 전 — **1회만** max(차단 상한, 바닥 + 1)에서 다시 clear 해 본다.
    Probe,
    /// 직전 clear 세션도 그랬다(확인) — 자동 clear 중단 · 바닥 + [`CTX_FLOOR_MIN_GROWTH`] 까지 실제로 자랄 때만 발화.
    Stopped,
}

impl CtxFloorRegime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raise => "raise",
            Self::Limited => "limited",
            Self::Backlog => "backlog",
            Self::Probe => "probe",
            Self::Stopped => "stopped",
        }
    }

    /// 선제 압축 천장 안에서 여유를 줄 수 없었다(오너 경고 대상 · `context.threshold` 의 `floor_limited`).
    pub fn floor_limited(self) -> bool {
        self != Self::Raise
    }

    fn tier3(self) -> bool {
        matches!(self, Self::Probe | Self::Stopped)
    }
}

/// ★(RR3-R1-1 · G3ROLE-1) clear 뒤 세션의 발화 임계(히스테리시스) — 순수 · 핀 `ctx_floor_bar_*`.
/// `floor` = 잰 바닥(%) · `base` = 역할 임계 · `confirmed` = 직전 clear 세션의 바닥도 차단 상한 근처였다(Probe 소진).
/// 반환 = (실효 임계 ≥ base, 영역). 불변식: 실효 임계 > base 이면 실효 임계 ≥ 바닥 + 1(Probe) · 그 밖은 ≥ 바닥 +
/// [`CTX_FLOOR_MIN_ROOM`] · 실효 임계 ≤ max(base, 바닥 + [`CTX_FLOOR_ROOM`]) 또는 [`CTX_FLOOR_NEVER`](바닥 > 90).
pub fn ctx_floor_bar(floor: u8, base: u8, window: Option<u64>, confirmed: bool) -> (u8, CtxFloorRegime) {
    let f = u16::from(floor.min(100));
    let c = u16::from(ctx_floor_ceiling(window));
    let h = u16::from(ctx_floor_hard_cap(window));
    let (room, min_room) = (u16::from(CTX_FLOOR_ROOM), u16::from(CTX_FLOOR_MIN_ROOM));
    let (bar, regime) = if f + min_room <= c {
        ((f + room).min(c), CtxFloorRegime::Raise)
    } else if f + min_room <= h {
        ((f + room).min(h), CtxFloorRegime::Limited)
    } else if !confirmed {
        (h.max(f + 1), CtxFloorRegime::Probe)
    } else {
        (f + u16::from(CTX_FLOOR_MIN_GROWTH), CtxFloorRegime::Stopped)
    };
    let bar = bar.min(u16::from(CTX_FLOOR_NEVER)) as u8;
    (bar.max(base), regime)
}

/// clear 뒤 세션의 발화 판정 한 벌 — [`ctx_floor_bar_measured`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxFloorBar {
    /// 실효 임계(≥ 기본).
    pub bar: u8,
    pub regime: CtxFloorRegime,
    /// 영역을 정한 바닥(복원 끝 바닥이 차단기 영역을 풀었으면 그것 · 아니면 정착 창 최고치) — 고지·payload 의 `floor_pct`.
    pub floor: u8,
    /// 정착 창 최고치(막대의 기준 — 유휴 재발화 0 은 이 값 위에서 지킨다).
    pub settled: u8,
    /// ★(RV2NC-E20-1) 얇은 세션 — 복원 끝에 여유가 있지만 정착 창 최고치가 차단 상한 근처(최고치 + 최소 여유 > 차단 상한).
    /// 그 발화는 성장 속도로 가른다([`CTX_FLOOR_THIN_SECS`]).
    pub thin: bool,
    /// ★(RV2NC-E20-2) 복원 턴 도중 바쁨을 보지 않는 기계 주입(채널 행·스케줄 직접 push·CEO 배달)이 들어와 복원 끝이 그 입력의
    /// 턴과 섞였다 — 복원 끝 바닥은 그 입력 때의 최고치(하한)로만 차단기를 판정했다(보고 바닥 = 정착 창 최고치).
    pub tainted: bool,
    /// ★(ADV2-R1-1) Claude 압축 뒤 재무장한 세션의 막대를 끌어올린 **clear 로 돌아오는 바닥**(압축 전 clear 세션의 정착 창
    /// 최고치) — 막대 = max(압축 뒤 막대, 이 값 + 1). 그보다 낮은 발화는 clear 가 컨텍스트를 오히려 올린다. 이 값이 선제 압축점
    /// 높이(200K 83%)면 막대 = [`CTX_FLOOR_NEVER`] — 그 clear 는 곧바로 다음 압축을 부른다(압축 뒤 cys 사이클 없음).
    pub lift: Option<u8>,
}

/// ★(ROLE-R4-1 · R2NC5-1) 잰 바닥 **둘**로 발화 판정을 정한다 — 순수 · 핀 `ctx_floor_bar_measured_*`.
/// `settled` = 정착 창 최고치(종전 잰 바닥 — 막대의 기준) · `restore` = 복원 턴 끝의 바닥([`CtxLoopGuard::restore_peak`] —
/// 복원 턴 뒤 첫 좌석 입력으로 확정됐을 때만).
///
/// 【왜】 사이클은 clear·재주입 동안 대기열·채널·스케줄 배달을 붙잡는다(`MachineHold::Quiescing`). 붙여넣기 직후 quiescing 이
/// 풀리면 붙잡혔던 배달이 짧은 복원 턴(라이브 master 14~53초) 끝 + 조용함 3초에 풀리고, 정착 창(최소 60초)은 그 턴들까지
/// 최고치에 싣는다 — 참 바닥 72 인 여유 있는 200K master 가 76~78 로 재여 Probe → Stopped(바닥+10 = 86~88 · Claude 선제
/// 압축점 83.5% 위)로 가고, Stopped 는 새 세션이 없으면 다시 재지 않으므로 cys clear 가 영영 안 났다(② · ROLE-R4-1 ·
/// R2NC5-1). 차단기(Probe·Stopped)는 "clear 해도 **clear 사이클 자신의 입력**(붙여넣기 → 복원 턴)만으로 차단 상한 근처"
/// 인 좌석을 위한 것이다 — 그 판정은 복원 턴 끝의 바닥으로 해야 한다.
///
/// 【무엇】 복원 끝 바닥이 확정됐고 그 바닥에 차단 상한 안 여유가 있으면(정상·Limited) 차단기 영역이 아니다: 정착 창
/// 최고치가 Probe·Stopped 높이여도 영역은 Limited · 확인(Stopped) 없음. 막대는 **정착 창 최고치의 미확인 막대**(= Probe 막대
/// max(차단 상한, 최고치+1)) 그대로다 — 복원 끝 판정이 틀려도(낮게 잡혀도) 막대는 종전 미확인 막대보다 낮아지지 않는다(유휴
/// 재발화 0 은 정착 창 최고치 위에서 그대로 · ①). 정착 창은 확정 뒤 몰림이 끝날 때까지 열려 있으므로([`CTX_FLOOR_BURST_QUIET_SECS`])
/// 붙잡혔던 배달만으로는 발화하지 않는다. 불변식: 미확인 막대 ≤ 반환 막대 ≤ 종전 막대(확인 반영) — 이 판정은 Stopped 를 풀 수만
/// 있고 어떤 막대도 앞당기지 않는다. 복원 끝 바닥이 차단기 높이면(참으로 가득 찬 좌석) 막대·영역은 종전 그대로(Probe 1회 →
/// Stopped). 복원 끝 바닥이 없으면(유휴 · 입력 없음 · 조용함 신호 없음) 종전 그대로. 보고 바닥(`floor` — 오너 feed·payload)은
/// 확정된 복원 끝이다(없으면 정착 창 최고치) — 배달을 바닥이라 부르지 않는다(R2NC5-1: 참 바닥 72 좌석에 '바닥 76% 로 돌아옴').
///
/// ★(RV2NC-E20-1 · ①) **얇은 세션**(복원 끝 여유 · 정착 창 최고치 + 최소 여유 > 차단 상한)은 확인을 영구히 끄지 않는다 —
/// e20f1cb6 은 여유 있는 복원 끝이 확인(`confirmed`)을 늘 지워, 사이클마다 몰림(사이클이 붙잡은 배달·사이클이 부른 회신)이
/// 최고치를 다시 채우는 유휴 좌석이 매 세션 최고치 + 1~4%p(주기 신호)에서 다시 발화했다(드릴 rv-d1 · rv-d1p — 1시간 10회).
/// · 미확인: 막대 = max(차단 상한, 최고치 + **2**), 영역 Limited — 1%p 반올림 해상도에서 발화가 실제 성장 ≥ 1%p 를 입증하는
///   최소 막대다(최고치 + 1 은 실제 성장 0.01%p 로도 닿는다 · 그 발화의 성장 속도를 가를 수 없다). 발화는 성장 속도로 가른다
///   ([`CtxLoopGuard::on_crossing`] · [`CTX_FLOOR_THIN_SECS`]) — 느리면 이 세션이 다음 세션의 확인 재료가 된다.
/// · 확인(`confirmed` — 직전 clear 세션이 차단 상한 근처였다): 막대 = 최고치 + [`CTX_FLOOR_MIN_ROOM`] + 1(표시 성장 − 1 이
///   최소 여유 — 사이클 하나가 실제 성장 5%p 로 값을 치른다), 영역 [`CtxFloorRegime::Backlog`] — 몰림을 바닥으로 확정한다(Stopped
///   의 + 10 이 아니다: 최고치 76~77 좌석은 선제 압축점 83.5% 아래에서 clear 되고 · 자동 압축을 끈 좌석도 최고치 82 까지는 차단점
///   88.5% 아래에서 발화한다).
/// 불변식(전수 핀): 미확인 막대 ≤ 반환 막대 · 반환 막대 ≤ 종전 막대(확인 반영) + 1(얇은 미확인의 +2 만 1%p 늦다) · 여유 있는
/// 복원 끝이면 Probe·Stopped 아님.
pub fn ctx_floor_bar_measured(settled: u8, restore: Option<u8>, base: u8, window: Option<u64>, confirmed: bool) -> CtxFloorBar {
    let restore = restore.map(|r| r.min(settled));
    let roomy = restore.is_some_and(|r| !ctx_floor_bar(r, base, window, false).1.tier3());
    let (bar, regime) = ctx_floor_bar(settled, base, window, confirmed && !roomy);
    // 보고 바닥 = 복원 끝(확정됐으면) — 오너에게 '바닥' 으로 말하는 값은 clear 사이클 자신의 입력이 끝난 높이다.
    let floor = restore.unwrap_or(settled);
    let (bar, regime, thin) = if roomy && regime.tier3() {
        let s = u16::from(settled);
        let (b, r) = if confirmed {
            // + 1: 표시 성장 − 1(반올림을 뺀 실제 성장의 하한)이 최소 여유 — 사이클 하나가 실제 성장 5%p 로 값을 치른다.
            (s + u16::from(CTX_FLOOR_MIN_ROOM) + 1, CtxFloorRegime::Backlog)
        } else {
            (u16::from(ctx_floor_hard_cap(window)).max(s + 2), CtxFloorRegime::Limited)
        };
        ((b.min(u16::from(CTX_FLOOR_NEVER)) as u8).max(base), r, true)
    } else {
        (bar, regime, false)
    };
    CtxFloorBar { bar, regime, floor, settled, thin, tainted: false, lift: None }
}

/// 발화 1건의 판정 재료(`context.threshold` payload) — clear 뒤 세션이 아니면(부트 첫 교차 등) 모두 None/false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxFire {
    pub floor: Option<u8>,
    pub regime: Option<CtxFloorRegime>,
    /// 정착 창 안 뒷문 발화(차단 상한 또는 학습한 바닥 상한 + 최소 여유를 넘음 — [`CtxLoopGuard::effective_threshold`]).
    pub settle_backstop: bool,
    /// 정착 창 최고치 — `floor` 와 다르면(복원 뒤 작업이 정착 창에 들었다) payload `settled_pct`.
    pub settled: Option<u8>,
    /// ★(R2NC5-1 (b)) Claude 자체 압축을 본 뒤 기본 임계로 재무장한 세션의 발화([`CtxLoopGuard::rearmed`]).
    pub after_compaction: bool,
}

/// ★(R2NC5-1 (b)) Claude 자체 압축 관측으로 재무장한 1건 — `context.floor_rearmed` 재료.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxRearm {
    /// 버린 바닥(복원 끝 바닥 또는 정착 창 최고치)과 그 영역.
    pub floor: u8,
    pub regime: CtxFloorRegime,
}

impl CtxFire {
    pub fn floor_limited(&self) -> bool {
        self.regime.is_some_and(CtxFloorRegime::floor_limited)
    }
}

/// 보류(기본 임계는 넘었지만 바닥 + 여유 아래) 고지 1건 — 이벤트는 세션 안 영역 상승마다(같은 (영역, 임계)는 한 번) ·
/// 오너 feed 는 정착 창이 닫힌 뒤 세션당 1번.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxHoldNotice {
    pub floor: u8,
    /// 정착 창 최고치(`floor` 와 다르면 복원 뒤 작업이 정착 창에 들었다).
    pub settled: u8,
    pub bar: u8,
    pub regime: CtxFloorRegime,
    pub ceiling: u8,
    pub hard_cap: u8,
    /// `context.floor_raised` 이벤트를 낸다(좌석의 직전 고지와 (영역, 임계)가 다르다).
    pub event: bool,
    /// 오너 feed 종류(없으면 무음) — Raise 는 좌석당 1회 · Limited·Probe·Stopped 는 1·2·4·8…번째 세션.
    pub feed: Option<&'static str>,
    /// 이 영역(Limited·Probe 합산 / Stopped)에 든 세션 수(오너 문안용).
    pub count: u32,
    /// ★(R1-RUNAWAY-1) 바닥이 clear 뒤가 아니라 Claude 자체 압축 뒤(지침 재읽기 포함)에 다시 잰 것이다(오너 문안용).
    pub after_compaction: bool,
    /// ★(RV2NC-E20-2) 복원 턴 도중 기계 주입이 섞여 복원 끝을 하한으로만 쟀다(오너 문안용 · [`CtxFloorBar::tainted`]).
    pub tainted: bool,
    /// ★(ADV2-R1-1) 압축 뒤 막대를 끌어올린 clear 로 돌아오는 바닥([`CtxFloorBar::lift`]).
    pub lift: Option<u8>,
}

/// 좌석별 clear 직후 바닥 가드(순수 상태 — 시각은 호출자가 단조 초로 준다). `Surface::ctx_loop_guard`.
#[derive(Debug, Default, Clone)]
pub struct CtxLoopGuard {
    /// 마지막 `context.threshold` 발화 시각.
    pub fired_at: Option<f64>,
    /// 마지막 발화 **뒤 첫** 세션 교체 시각(이 세션 = clear 뒤 세션). Stopped 좌석은 그 뒤 교체도 다시 잰다.
    pub reset_at: Option<f64>,
    /// 정착 창의 시작점 — 교체 시각에서 시작해 quiescing(사이클 진행 중) 관측이 뒤로 민다(교체 + 정착 창까지).
    pub settle_anchor: Option<f64>,
    /// 정착 창 안에서 관측한 최고치 = 이 세션의 바닥(`floor_window` 기준 퍼센트).
    pub settle_peak: Option<u8>,
    /// 바닥이 정착 창 안에서 마지막으로 오른 시각 — 기본 창 뒤에도 [`CTX_FLOOR_SETTLE_QUIET_SECS`] 안에 올랐으면 연장.
    pub settle_rise_at: Option<f64>,
    /// 정착 창 안 마지막 관측 시각(시작점 뒤 관측이 있어야 조용함으로 창을 닫는다 — 붙여넣기 턴의 보고 전 닫힘 금지).
    pub settle_seen_at: Option<f64>,
    /// 정착 창이 조용함(복원 턴 끝)으로 닫혔다 — 그 뒤 관측은 작업이다([`Self::note_idle`]).
    pub settle_closed: bool,
    /// 바닥을 잰 창(창이 바뀌면 바닥을 토큰 비율로 옮긴다 · 결측은 값이 아니다).
    pub floor_window: Option<u64>,
    /// 이 세션이 차단 상한 근처(Probe·Stopped 바닥 또는 정착 창 뒷문 발화)였다.
    pub session_tier3: bool,
    /// 직전에 잰 clear 세션이 그랬다 — 이번에도 그러면 확인(Stopped).
    pub prev_tier3: bool,
    /// 정착 창 뒷문을 이미 한 번 썼다 — 다음 clear 세션에는 뒷문이 없다(붙여넣기·복원만으로 차단 상한을 넘는 유휴 좌석이
    /// 세션마다 뒷문으로 도는 고리 차단 · ①). 정착 창 **밖**의 정상·Limited 발화(바닥이 차단 상한에서 먼 좌석의 실제
    /// 성장)가 나면 다시 무장한다.
    pub backstop_spent: bool,
    /// 이 세션에서 이미 고지한 가장 높은 영역 · 이 세션의 오너 feed 를 이미 판정했다(정착 창이 닫힌 뒤 1번).
    pub session_notice: Option<CtxFloorRegime>,
    pub session_fed: bool,
    /// 좌석의 마지막 `context.floor_raised` (영역, 임계) — 사이클마다 같은 고지 반복 금지.
    pub last_announced: Option<(CtxFloorRegime, u8)>,
    /// Raise 오너 feed 를 이미 냈다(좌석당 1회).
    pub raise_fed: bool,
    /// Limited·Probe 에 든 세션 수 / Stopped 에 든 세션 수.
    pub limited_count: u32,
    pub stopped_count: u32,
    /// ★(ROLE-R4-1 · R2NC5-1) 복원 턴이 보고됐다 — 정착 창 시작점 뒤 첫 **상승** 관측 시각(붙여넣기를 처리한 턴의 보고).
    /// 시작점이 이 뒤로 밀리면 지운다(그 상승은 이 붙여넣기의 것이 아니다).
    pub restore_rise_at: Option<f64>,
    /// 복원 턴 끝 후보 — 상승 관측 뒤 처음 온 ≥ [`CTX_FLOOR_IDLE_QUIET_SECS`] 조용함의 시작 시각.
    pub restore_quiet_from: Option<f64>,
    /// 복원 턴 끝의 바닥(차단기 영역 판정용) — 후보 조용함 **뒤 첫 좌석 입력**(대기열·채널·스케줄 배달 · 직접 send · 사람
    /// 입력)이 쓰인 순간의 정착 창 최고치. 그 뒤 관측은 새 턴(복원 뒤 작업)이다. 입력 없이 출력이 다시 흐르면(틈이 턴 끝이
    /// 아니었다 — 같은 턴이 이어짐) 확정하지 않는다 — 정착 창 최고치를 그대로 따른다(종전 거동 · 실패 방향).
    pub restore_peak: Option<u8>,
    /// `restore_peak` 를 잰 창(창이 바뀌면 토큰 비율로 옮긴다).
    pub restore_window: Option<u64>,
    /// 복원 끝 바닥을 확정한 입력 시각.
    pub restore_freeze_at: Option<f64>,
    /// 대기열 인계로 확정했을 때만 — 그 배달의 제출(CR) 전 · [`CTX_FLOOR_INPUT_GRACE_SECS`] 안의 보고를 복원 턴의 늦은 보고로
    /// 싣는 기한(인계 시각 + 유예).
    pub restore_grace_until: Option<f64>,
    /// 좌석 writer 의 마지막 Inject 끝(CR 기록) 시각([`CtxIdleObs::inject_done`]).
    pub inject_done_at: Option<f64>,
    /// 좌석에 마지막으로 입력이 쓰인 시각(데몬 단조 초 · [`CtxIdleObs::last_input`]) — 세션과 무관한 좌석 사실.
    pub input_at: Option<f64>,
    /// 좌석의 마지막 **대기열 배달** 시각([`CtxIdleObs::last_queue_delivery`]) — 대기열은 턴이 끝난 좌석(프롬프트 경계 ·
    /// 바쁨 표지 없음 · 마커 없는 좌석은 조용함 3초)에만 배달하므로, 복원 턴의 보고 뒤 첫 배달은 복원 턴 끝의 증거다.
    pub queue_at: Option<f64>,
    /// ★(R2NC6-1 · RV-ROLE-CF42-1) 좌석 대기열에 남은 가장 오래된 항목의 enqueue 시각(데몬 단조 초 · 없으면 빈 대기열 ·
    /// [`CtxIdleObs::queue_oldest`] 최신 관측) — 정착 창 시작점(사이클 quiescing 해제) **전**에 들어온 항목이 남아 있으면 사이클이
    /// 붙잡았던 배달 몰림이 아직 끝나지 않았다([`Self::held_pending`]). 시작점 뒤에 들어온 항목은 새 작업이다.
    pub queue_oldest: Option<f64>,
    /// ★(R2NC5-1 (b)) Claude 자체 압축(같은 세션 파일에서 컨텍스트가 잰 바닥보다 [`CTX_FLOOR_COMPACT_DROP`] 이상 떨어짐)을
    /// 봤다 — 이 세션의 잰 바닥을 버리고 재무장했다. ★(R1-RUNAWAY-1) 재무장은 압축 관측 시각부터 바닥을 **다시 잰다**(정착 창을
    /// 새로 연다 · 뒷문 없음) — 발화는 압축 뒤 바닥 위로 실제로 자란 뒤다. 다음 세션 교체(발화 없는 교체 포함)가 다시 잰다.
    pub rearmed: bool,
    /// 마지막 세션 교체 통지(잰 것·무시한 것 모두 — 수집기의 늦은 중복 통지 포함) 뒤 관측 최고치와 그 창 — 압축 판정은 **같은
    /// 세션** 안의 낙폭만 본다(발화 없이 바뀐 새 세션의 낮은 첫 보고는 압축이 아니다 · 결측은 값이 아니다).
    pub session_peak: Option<u8>,
    pub session_peak_window: Option<u64>,
    /// ★(RV2NC-E20-1) 정착 창이 조용함으로 닫힌 시각 — 얇은 세션 발화의 성장 속도를 여기서부터 잰다(시간으로 끝난 창은 그 끝).
    pub settle_closed_at: Option<f64>,
    /// 마지막 관측 퍼센트와 그 창 — 발화 판정([`Self::on_crossing`])이 그 발화의 표시 높이를 본다.
    pub last_pct: Option<(u8, Option<u64>)>,
    /// ★(RV2NC-E20-1) 이 세션의 얇은 발화가 일하는 좌석의 빠른 성장이었다 — 다음 세션의 확인 재료에서 뺀다(확인을 푼다).
    pub session_fast: bool,
    /// ★(RV2NC-E20-2) 복원 턴 도중(시작점 뒤 · 복원 끝 전) 바쁨을 보지 않는 기계 주입이 들어왔다 — 복원 끝을 그 순간의 최고치
    /// (하한)로 확정했다. 그 입력은 Claude 가 복원 턴 뒤에 곧바로(조용함 없이) 이어 처리하므로 조용함·배달 신호로는 복원 끝과
    /// 그 입력의 턴을 가를 수 없다(드릴 rv-d3: 복원 끝 71.7 → 0.5초 뒤 채널 행 제출 → 76 이 복원 끝으로 재여 Probe → Stopped).
    pub restore_tainted: bool,
    /// 이 세션의 붙여넣기(시작점 뒤 첫 좌석 입력) 시각 — 복원 턴 오염 판정에서 붙여넣기 자신을 뺀다.
    pub paste_input_at: Option<f64>,
    /// ★(RV2NC-E20-1) 사이클 몰림의 기준 시각 — 복원 끝을 확정한 입력 시각(없으면 시작점). 이 전에 대기열에 들어온 항목은 복원
    /// 턴이 끝나기 전에 쌓인 것(사이클이 붙잡은 배달 · 복원 턴 동안 선 회신 — rv-d1p)이라 다 배달될 때까지 창을 연다.
    pub backlog_cutoff: Option<f64>,
    /// ★(ADV2-R1-1) Claude 압축으로 재무장한 세션의 **clear 로 돌아오는 바닥**(압축 전 clear 세션의 정착 창 최고치)과 그 창 —
    /// 압축 뒤 발화는 이 바닥 + 1 이상에서만(그보다 낮게 발화하면 cys clear 가 컨텍스트를 오히려 올린다 · 참으로 가득 찬 좌석의
    /// clear → 압축 → 재무장 고리).
    pub rearm_floor: Option<u8>,
    pub rearm_floor_window: Option<u64>,
    /// 재무장 전 세션이 얇은 세션(또는 Backlog)이었다 — 압축 뒤 발화도 성장 속도로 가른다.
    pub rearm_thin: bool,
    /// ★(ADV2-R1-1) clear 뒤 정착 창 안에서 **사이클 자신의 입력**(붙여넣기·복원 턴 · 사이클이 붙잡은 몰림)이 Claude 선제 압축을
    /// 불렀다(창 최고치보다 낙폭 이상 낮은 관측 — 창 최고치는 압축점에서 잘린 값이다) — 그 clear 는 선제 압축점 위로 돌아온다.
    pub window_compacted: bool,
    /// 이 세션의 정착 창에 사이클이 붙잡은 대기열 몰림이 있었다([`Self::held_pending`] 을 한 번이라도 봤다) — 몰림의 마지막 턴
    /// (대기열은 이미 비었지만 창이 아직 열린 동안)에 온 압축도 사이클 자신의 입력이 부른 것이다.
    pub held_seen: bool,
    /// 정착 창 시작점 + [`CTX_FLOOR_SETTLE_MIN_SECS`] 까지의 최고치와 그 창 — 복원 끝 신호(입력·조용함)가 하나도 없는 좌석(입력
    /// 없이 끊김 없는 긴 턴)의 clear 로 돌아오는 바닥 추정(창 전체 최고치는 그 턴의 작업을 싣는다).
    pub early_peak: Option<u8>,
    pub early_peak_window: Option<u64>,
}

/// 퍼센트를 창 `from` 기준에서 `to` 기준으로 옮긴다(토큰 비율 · 올림 · 100 캡) — 창이 같거나 어느 쪽이 미상이면 그대로.
fn rebase_pct(v: u8, from: Option<u64>, to: Option<u64>) -> u8 {
    match (from, to) {
        (Some(fw), Some(w)) if fw != w && w > 0 => (u64::from(v).saturating_mul(fw).div_ceil(w)).min(100) as u8,
        _ => v,
    }
}

impl CtxLoopGuard {
    /// 지금 세션이 마지막 발화 뒤의 clear 세션인가.
    fn post_clear(&self) -> bool {
        matches!((self.fired_at, self.reset_at), (Some(f), Some(r)) if r >= f)
    }

    /// 잰 바닥을 `window` 기준 퍼센트로(창이 바뀌었으면 토큰 비율로 옮긴다 — 올림 · 100 캡).
    fn floor_for(&self, window: Option<u64>) -> Option<u8> {
        self.settle_peak.map(|f| rebase_pct(f, self.floor_window, window))
    }

    /// 확정된 복원 끝 바닥을 `window` 기준 퍼센트로.
    fn restore_floor_for(&self, window: Option<u64>) -> Option<u8> {
        self.restore_peak.map(|f| rebase_pct(f, self.restore_window, window))
    }

    /// clear 뒤 세션의 발화 판정 — 바닥 미상이면 None(기본 임계). Claude 자체 압축으로 재무장한 세션은 **압축 뒤 다시 잰
    /// 바닥**으로 판정한다(★R1-RUNAWAY-1 — [`Self::note_compaction`]).
    ///
    /// ★(RV2NC-E20-2) 복원 턴이 기계 주입으로 오염된 세션은 복원 끝(그 입력 때의 최고치 — 하한)으로 차단기만 판정하고 보고
    /// 바닥은 정착 창 최고치다. ★(ADV2-R1-1) 압축으로 재무장한 세션의 막대는 clear 로 돌아오는 바닥 + 1 아래로 내려가지 않는다.
    fn bar(&self, base: u8, window: Option<u64>) -> Option<CtxFloorBar> {
        if !self.post_clear() {
            return None;
        }
        let w = window.or(self.floor_window);
        let settled = self.floor_for(w)?;
        let mut b = ctx_floor_bar_measured(settled, self.restore_floor_for(w), base, w, self.prev_tier3);
        if self.restore_tainted && self.restore_peak.is_some() {
            b.tainted = true;
            b.floor = b.settled;
        }
        if let Some(f) = self.rearm_floor_for(w).filter(|_| self.rearmed) {
            // clear 로 돌아오는 바닥이 선제 압축점 높이면 그 clear 는 곧바로 다음 Claude 압축을 부른다(여유 0) — 압축 뒤 cys 사이클을
            // 걸지 않는다(Claude 압축이 받는다 · 자동 압축을 끈 좌석은 압축이 없어 이 길에 오지 않는다).
            let lift = if f >= ctx_pct_below_reserve(w, CC_SUMMARY_RESERVE_TOKENS + CC_AUTOCOMPACT_BUFFER_TOKENS) {
                u16::from(CTX_FLOOR_NEVER)
            } else {
                u16::from(f) + 1
            };
            if lift > u16::from(b.bar) {
                b.bar = lift.min(u16::from(CTX_FLOOR_NEVER)) as u8;
                b.lift = Some(f);
            }
        }
        Some(b)
    }

    /// 압축 전 clear 세션의 clear 로 돌아오는 바닥을 `window` 기준 퍼센트로.
    fn rearm_floor_for(&self, window: Option<u64>) -> Option<u8> {
        self.rearm_floor.map(|f| rebase_pct(f, self.rearm_floor_window, window))
    }

    /// ★(RV2NC-E20-1) 정착 창이 끝난 시각 — 조용함으로 닫혔으면 그 시각, 아니면 시간 창의 끝(압축 뒤 창은 [`CTX_FLOOR_REREAD_SECS`]).
    fn settle_end(&self) -> Option<f64> {
        if let Some(c) = self.settle_closed_at {
            return Some(c);
        }
        let a = self.settle_anchor?;
        if self.rearmed {
            return Some(a + CTX_FLOOR_REREAD_SECS);
        }
        let base = a + CTX_FLOOR_SETTLE_SECS;
        Some(match self.settle_rise_at {
            Some(r) if r + CTX_FLOOR_SETTLE_QUIET_SECS > base => (r + CTX_FLOOR_SETTLE_QUIET_SECS).min(a + CTX_FLOOR_SETTLE_MAX_SECS),
            _ => base,
        })
    }

    /// ★(RV2NC-E20-1) 얇은 세션의 이 발화가 **일하는 좌석**의 성장인가 — 정착 창이 끝난 뒤 발화까지 표시 성장 − 1(반올림을 뺀
    /// 실제 성장의 하한)이 [`CTX_FLOOR_THIN_SECS`] 에 최소 여유를 채우는 속도(분당 0.2%p) 이상. 재료가 없으면 아니다(느린 쪽 —
    /// 확인 재료가 된다 · 다음 세션은 몰림 위 최소 여유 성장마다 · ① 쪽 실패).
    fn thin_growth_fast(&self, window: Option<u64>, now: f64) -> bool {
        let w = window.or(self.floor_window);
        let (Some(s), Some((p, pw)), Some(from)) = (self.floor_for(w), self.last_pct, self.settle_end()) else { return false };
        let grown = u16::from(rebase_pct(p, pw, w)).saturating_sub(u16::from(s) + 1);
        f64::from(grown) * CTX_FLOOR_THIN_SECS >= f64::from(CTX_FLOOR_MIN_ROOM) * (now - from).max(1.0)
    }

    /// 정착 창 최고치만으로 본 영역(확인 없이) — 뒷문 재무장 판정(복원 끝 판정과 무관하게 종전 규칙).
    fn settled_regime(&self, window: Option<u64>) -> Option<CtxFloorRegime> {
        if !self.post_clear() {
            return None;
        }
        let w = window.or(self.floor_window);
        self.floor_for(w).map(|f| ctx_floor_bar(f, 0, w, false).1)
    }

    /// 복원 끝 바닥이 확정됐고 차단 상한 안에 여유가 있다(정상·Limited 높이) — 이 세션은 차단기 재료가 아니다.
    fn restore_roomy(&self, window: Option<u64>) -> bool {
        let w = window.or(self.floor_window);
        self.restore_floor_for(w).is_some_and(|r| !ctx_floor_bar(r, 0, w, false).1.tier3())
    }

    /// ★(RV2NC-E20-2 · ②) clear 사이클의 **복원 턴이 아직 도는 중**인가 — clear 뒤 세션 · 정착 창 안 · 복원 끝 미확정 · 복원 턴 끝
    /// 후보(≥2초 조용함)도 아직 없음 · 시작점 + [`CTX_FLOOR_RESTORE_HOLD_SECS`] 안. 채널 inbox 행·스케줄 직접 push 는 바쁨을 보지
    /// 않아, quiescing 이 풀린 직후 보류분이 복원 턴 **도중**에 타이핑되면 Claude 가 복원 턴 뒤 조용함 없이 곧바로 처리한다 —
    /// 그 턴이 복원 끝 바닥에 들어 여유 있는 master(참 바닥 72)가 76 으로 재여 Probe → Stopped 였다(드릴 rv-d3). 이 동안 그 두
    /// 생산자는 보류(채널 — 행 `new` 유지 · 15초 sweep 재시도)·좌석 큐 우회(스케줄 — 대기열은 턴이 끝난 좌석에만 배달)한다 —
    /// 대기열 배달과 같은 순서(복원 턴 끝 뒤)라 복원 끝이 오염되지 않는다. Claude 는 그 입력을 어차피 복원 턴 뒤에 처리하므로
    /// 늘어나는 지연은 조용함 판정(2초) + 재시도 간격뿐이다. 압축 뒤 재는 창은 복원 턴이 없다(보류 없음).
    pub fn restore_turn_open(&self, now: f64) -> bool {
        self.post_clear()
            && !self.rearmed
            && self.restore_peak.is_none()
            && self.restore_quiet_from.is_none()
            && self.settling(now)
            && self.settle_anchor.is_some_and(|a| now >= a && now - a <= CTX_FLOOR_RESTORE_HOLD_SECS)
    }

    /// ★(ROLE-R4-1 · R2NC5-1) 좌석에 입력이 쓰인 시각(데몬 단조 초 · [`CtxIdleObs::last_input`]) — 모든 경로(운영 두 경로 ·
    /// 수집기 틱)에서 관측·조용함 판정 **전에** 부른다. 복원 턴 끝 후보(조용함) 뒤 첫 입력이 복원 끝 바닥을 확정한다.
    pub fn note_input(&mut self, at: Option<f64>, now: f64) {
        if let Some(t) = at.filter(|t| t.is_finite()) {
            self.input_at = Some(self.input_at.map_or(t, |p| p.max(t)));
            // ★(RV2NC-E20-2) 시작점 뒤 첫 좌석 입력 = 이 세션의 붙여넣기(사이클이 붙여넣은 뒤 quiescing 을 푼다) — 복원 턴 오염
            //   판정에서 뺀다.
            if self.paste_input_at.is_none() && !self.rearmed && self.settle_anchor.is_some_and(|a| t > a) && self.settling(now) {
                self.paste_input_at = Some(t);
            }
        }
        self.freeze_restore(now);
    }

    /// ★(ROLE-R4-1 · R2NC5-1) 좌석의 마지막 대기열 배달 시각(데몬 단조 초 · [`CtxIdleObs::last_queue_delivery`]) — 입력이기도
    /// 하다. [`Self::note_input`] 과 같은 자리(관측·조용함 판정 전)에서 부른다.
    /// `inject_done` = writer 의 마지막 Inject 끝(CR 기록) — 인계한 배달이 실제로 제출된 시각(유예를 끝낸다).
    pub fn note_delivery(&mut self, at: Option<f64>, inject_done: Option<f64>, now: f64) {
        if let Some(t) = at.filter(|t| t.is_finite()) {
            self.queue_at = Some(self.queue_at.map_or(t, |p| p.max(t)));
            self.input_at = Some(self.input_at.map_or(t, |p| p.max(t)));
        }
        if let Some(t) = inject_done.filter(|t| t.is_finite()) {
            self.inject_done_at = Some(self.inject_done_at.map_or(t, |p| p.max(t)));
        }
        self.freeze_restore(now);
        self.note_taint(now);
    }

    /// ★(RV2NC-E20-2 · ②) 복원 턴 오염 — 붙여넣기 뒤·복원 끝(≥2초 조용함 · 대기열 배달) 전에 **대기열 배달이 아닌** writer Inject
    /// (채널 행 · 스케줄 직접 push · CEO 배달 — 바쁨을 보지 않는다 · quiescing 해제 직후 보류분이 몰려 온다)가 끝났으면, Claude 는 그
    /// 입력을 복원 턴 뒤에 조용함 없이 곧바로 처리한다 — 복원 끝과 그 입력의 턴을 가를 신호가 없다. 그때까지의 최고치(복원 끝의
    /// **하한**)로 복원 끝을 확정하고 오염으로 적는다: 차단기(Probe·Stopped)는 그 하한이 차단 상한 근처일 때만(참으로 가득 찬 좌석),
    /// 아니면 얇은 세션(몰림 = 정착 창 최고치 · 성장 속도로 가른다 — ① 은 거기서 묶인다). 종전(e20f1cb6)은 그 입력의 턴을 복원
    /// 끝에 실어 여유 있는 master(참 바닥 72)가 76 으로 재여 Probe → Stopped(86 · 선제 압축점 위) — 매 세션 Claude 압축이 cys
    /// clear 보다 먼저 왔고 자동 압축을 끈 좌석은 막대 89 가 차단점 88.5% 위였다(드릴 rv-d3·rv-d3n).
    fn note_taint(&mut self, now: f64) {
        if self.restore_peak.is_some() || self.restore_quiet_from.is_some() || self.rearmed || !self.settling(now) {
            return;
        }
        let (Some(d), Some(a)) = (self.inject_done_at, self.settle_anchor) else { return };
        let after_paste = d > a && self.paste_input_at.is_some_and(|p| d > p);
        let queued = self.queue_at.is_some_and(|q| d >= q && d <= q + CTX_FLOOR_QUEUE_CR_SECS);
        if !after_paste || queued {
            return;
        }
        self.restore_peak = Some(self.settle_peak.unwrap_or(0));
        self.restore_window = self.floor_window;
        self.restore_freeze_at = Some(d);
        self.restore_grace_until = None;
        self.restore_tainted = true;
    }

    /// ★(R2NC6-1 · RV-ROLE-CF42-1) 좌석 대기열에 남은 가장 오래된 항목의 enqueue 시각(데몬 단조 초 · None = 빈 대기열 ·
    /// [`CtxIdleObs::queue_oldest`]) — 조용함 판정 **전**에 부른다(최신 관측으로 덮는다).
    pub fn note_queue(&mut self, oldest: Option<f64>) {
        self.queue_oldest = oldest.filter(|t| t.is_finite());
        if self.held_pending() && self.post_clear() && !self.rearmed && !self.settle_closed {
            self.held_seen = true;
        }
    }

    /// 사이클이 붙잡았던 대기열 항목(정착 창 시작점 = quiescing 해제 전에 들어온 것)이 아직 배달되지 않았다.
    /// ★(RV2NC-E20-1) 기준은 시작점이 아니라 **복원 끝을 확정한 입력 시각**([`Self::backlog_cutoff`])까지 — 복원 턴 동안 대기열에
    /// 선 항목(복원된 좌석이 부른 회신 몰림 · 드릴 rv-d1p: 붙여넣기 턴 끝 2초 뒤 3건)도 사이클의 몰림이다. 종전(시작점만)은 그
    /// 몰림 한가운데서 창을 닫아 나머지를 성장으로 세어, 회신만으로 매 세션 발화했다(1시간 11회).
    fn held_pending(&self) -> bool {
        let cutoff = |a: f64| self.backlog_cutoff.map_or(a, |c| c.max(a));
        matches!((self.queue_oldest, self.settle_anchor), (Some(o), Some(a)) if o <= cutoff(a))
    }

    /// 복원 끝 바닥 확정 — 둘 중 먼저 성립하는 것(정착 창 안에서만):
    /// (가) 복원 턴의 보고(시작점 뒤 상승) **뒤에 온 대기열 배달** — 대기열은 턴이 끝난 좌석에만 배달한다(프롬프트 경계 ·
    ///      바쁨 표지 없음). claude 좌석의 프롬프트 경계 배달은 조용함을 기다리지 않아 복원 턴 끝과 첫 배달 사이에 2초 틈이
    ///      없을 수 있다(샌드박스 실측 0.1~0.5초) — 틈만으로는 그 배달 턴이 복원 끝 바닥에 든다.
    /// (나) 후보 조용함(≥2초 · 복원 턴 끝) **시작 뒤**의 좌석 입력(직접 send · 사람 입력 · 채널 행 등).
    /// 확정 값 = 그 순간까지의 정착 창 최고치(그 입력이 부른 턴의 보고는 아직 싣지 않았다 — 호출 순서가 관측 전이다) ·
    /// (가)로 확정했으면 그 배달의 제출(CR) 전 · [`CTX_FLOOR_INPUT_GRACE_SECS`] 안의 보고는 복원 턴의 늦은 보고로 싣는다
    /// ((나)는 입력 시각이 이미 제출 뒤라 유예가 없다). 입력 없이 출력이 다시 흐른 틈(같은 턴이 이어짐)은 확정하지 않는다 —
    /// 다음 입력 때 그때까지의 최고치로 확정한다.
    fn freeze_restore(&mut self, now: f64) {
        // ★(RV2NC-E20-2) 오염으로 확정한 세션의 몰림 기준 — 복원 턴 뒤 첫 대기열 배달(복원 턴이 끝났다는 증거)에서 선다.
        if self.restore_tainted && self.backlog_cutoff.is_none() {
            if let (Some(q), Some(f)) = (self.queue_at, self.restore_freeze_at) {
                if q > f && self.settling(now) {
                    self.backlog_cutoff = Some(q);
                }
            }
        }
        // 압축 뒤 재는 창(R1-RUNAWAY-1 · RV2-ROLE-1)에는 복원 턴이 없다 — 압축 뒤 바닥은 짧은 창의 최고치다.
        if self.restore_peak.is_some() || self.rearmed || !self.settling(now) {
            return;
        }
        let Some(p) = self.settle_peak else { return };
        let by_queue = match (self.queue_at, self.restore_rise_at) {
            (Some(d), Some(r)) if d > r => Some(d),
            _ => None,
        };
        let by_gap = match (self.restore_quiet_from, self.input_at) {
            (Some(q), Some(i)) if i > q => Some(i),
            _ => None,
        };
        let (at, grace) = match (by_queue, by_gap) {
            (Some(a), Some(b)) if b < a => (b, None),
            (Some(a), _) => (a, Some(a + CTX_FLOOR_INPUT_GRACE_SECS)),
            (None, Some(b)) => (b, None),
            (None, None) => return,
        };
        self.restore_peak = Some(p);
        self.restore_window = self.floor_window;
        self.restore_freeze_at = Some(at);
        self.restore_grace_until = grace;
        self.backlog_cutoff = Some(at);
    }

    /// 세션 교체(clear·새 세션) 관측 — 마지막 발화 **뒤 첫** 교체가 정착 창을 연다(발화 이력이 없으면 무시). 자동 clear 를
    /// 멈춘(Stopped) 좌석은 그 뒤 교체(오너의 수동 clear·재기동)도 다시 잰다 — 처방 뒤 바닥이 내려가면 곧바로 정상 영역.
    pub fn note_session_change(&mut self, now: f64) {
        // 압축 판정의 '같은 세션' 기준은 어떤 교체 통지에서든 새로 시작한다(아래에서 잰 바닥을 유지하는 교체라도).
        self.session_peak = None;
        self.session_peak_window = None;
        let Some(fired) = self.fired_at else { return };
        // ★(RV2NC-E20-1) 몰림을 바닥으로 확정한 세션(Backlog)도 Stopped 처럼 그 뒤 교체(오너 clear)를 다시 잰다.
        let stopped = self.bar(0, None).is_some_and(|b| matches!(b.regime, CtxFloorRegime::Stopped | CtxFloorRegime::Backlog));
        // ★(R2NC5-1 (b)) Claude 압축으로 재무장한 세션도 그 뒤 교체(발화 없는 오너 clear 포함)를 다시 잰다.
        if self.reset_at.is_some_and(|r| r >= fired) && !stopped && !self.rearmed {
            return;
        }
        // 직전에 잰 clear 세션의 영역을 넘긴다(연속 확인 재료). 잰 적이 없으면(부트 첫 발화 뒤) 확인 아님. ★(RV2NC-E20-1) 얇은
        // 세션의 발화가 일하는 좌석의 빠른 성장이었으면 확인 재료가 아니다(바쁜 여유 좌석은 차단 상한 근처에서 계속 clear · ②).
        self.prev_tier3 = self.reset_at.is_some() && self.session_tier3 && !self.session_fast;
        self.session_fast = false;
        self.settle_closed_at = None;
        self.restore_tainted = false;
        self.paste_input_at = None;
        self.backlog_cutoff = None;
        self.rearm_floor = None;
        self.rearm_floor_window = None;
        self.rearm_thin = false;
        self.window_compacted = false;
        self.held_seen = false;
        self.early_peak = None;
        self.early_peak_window = None;
        self.reset_at = Some(now);
        self.settle_anchor = Some(now);
        self.settle_peak = None;
        self.settle_rise_at = None;
        self.settle_seen_at = None;
        self.settle_closed = false;
        self.floor_window = None;
        self.session_tier3 = false;
        self.session_notice = None;
        self.session_fed = false;
        self.restore_rise_at = None;
        self.restore_quiet_from = None;
        self.restore_peak = None;
        self.restore_window = None;
        self.restore_freeze_at = None;
        self.restore_grace_until = None;
        self.rearmed = false;
    }

    /// 사이클 진행 중(quiescing) 관측 — 정착 창의 시작점을 뒤로 민다(재주입·붙여넣기가 끝난 뒤부터 복원을 잰다).
    /// 교체 시각 + 정착 창까지만(멈춘 quiescing 이 창을 무한히 열어 두면 발화가 막힌다 — ②).
    pub fn note_quiescing(&mut self, now: f64) {
        if !self.post_clear() {
            return;
        }
        if let (Some(r), Some(a)) = (self.reset_at, self.settle_anchor) {
            if now >= a && now - a <= CTX_FLOOR_SETTLE_SECS {
                let anchor = now.min(r + CTX_FLOOR_SETTLE_SECS);
                self.settle_anchor = Some(anchor);
                // 시작점이 밀렸다 — 그 앞의 상승·턴 끝 후보는 이 붙여넣기의 복원 턴 것이 아니다.
                if self.restore_rise_at.is_some_and(|t| t < anchor) {
                    self.restore_rise_at = None;
                }
                if self.restore_quiet_from.is_some_and(|t| t < anchor)
                    || self.restore_freeze_at.is_some_and(|t| t < anchor)
                {
                    self.restore_quiet_from = None;
                    self.restore_peak = None;
                    self.restore_window = None;
                    self.restore_freeze_at = None;
                    self.restore_grace_until = None;
                    self.restore_tainted = false;
                    self.backlog_cutoff = None;
                }
                if self.paste_input_at.is_some_and(|t| t <= anchor) {
                    self.paste_input_at = None;
                }
                self.early_peak = None;
            }
        }
    }

    /// 지금이 clear 뒤 세션의 정착 창 안인가 — 시작점 + [`CTX_FLOOR_SETTLE_SECS`], 그 뒤에도 바닥이 최근
    /// [`CTX_FLOOR_SETTLE_QUIET_SECS`] 안에 올랐으면(복원이 아직 도는 중) 시작점 + [`CTX_FLOOR_SETTLE_MAX_SECS`] 까지.
    /// 그 전에라도 좌석이 조용해지면(복원 턴 끝 — [`Self::note_idle`]) 닫힌다.
    pub fn settling(&self, now: f64) -> bool {
        if !self.post_clear() || self.settle_closed {
            return false;
        }
        let (Some(r), Some(a)) = (self.reset_at, self.settle_anchor) else { return false };
        if now < r {
            return false;
        }
        // ★(RV2-ROLE-1) 압축 뒤 재는 창은 지침 재읽기 몫만 — 연장·최소 창·조용함 닫힘 없음.
        if self.rearmed {
            return now - a <= CTX_FLOOR_REREAD_SECS;
        }
        now - a <= CTX_FLOOR_SETTLE_SECS
            || (now - a <= CTX_FLOOR_SETTLE_MAX_SECS
                && self.settle_rise_at.is_some_and(|t| now - t <= CTX_FLOOR_SETTLE_QUIET_SECS))
    }

    /// ★(자기 반례 · 차단기 오판) 좌석 출력이 `idle_from` ~ `idle_to` 동안 조용했다(턴 끝) — 이 조용함이 정착 창 시작점
    /// 뒤에 시작해 최소 창([`CTX_FLOOR_SETTLE_MIN_SECS`]) 이후까지 이어졌고 그 사이 관측이 있었으면 **정착 창을 닫는다**:
    /// clear 사이클 자신의 입력(붙여넣기 → 복원 턴)이 끝난 높이가 바닥이고, 그 뒤 들어온 작업(대기열 배달·push 한 턴씩)은
    /// 바닥이 아니라 성장이다. 시간 창만 쓰면 바쁜 좌석의 바닥이 복원 뒤 작업만큼 부풀어(200K master 72 → 77~85) 여유 있는
    /// 좌석이 Probe·Stopped(바닥+10 = 선제 압축점 위)로 오판된다(② — cys clear 가 Claude 압축에 가린다).
    /// 호출 순서(운영 두 경로·수집기 틱 공통): **끝난 틈은 관측 전**(틈 뒤 보고는 새 턴의 것 — 싣지 않는다) · **이어지는
    /// 조용함은 관측 뒤**(조용한 동안 온 보고는 직전 턴의 끝이다). 실패 방향: 조용함 신호가 없으면(출력이 끊이지 않는 좌석 ·
    /// 수집기 정지) 종전 시간 창 그대로 — 닫힘이 늦어질 뿐 이르지 않다. 닫힌 뒤에는 정착 창 뒷문도 없다(막대가 차단 상한 이하).
    ///
    /// ★(ROLE-R4-1 · R2NC5-1) 같은 조용함이 **복원 턴 끝 후보**도 정한다 — 시작점 뒤 상승 관측(복원 턴의 보고)이 이 조용함의
    /// 끝 이전에 있었고 조용함이 시작점 뒤에 시작했으면(최소 창과 무관 — 사이클이 붙잡았던 배달은 복원 턴 끝 + 3초에 오고
    /// 최소 창보다 이르다). 후보 뒤 첫 좌석 입력이 복원 끝 바닥을 확정한다([`Self::note_input`]).
    pub fn note_idle(&mut self, idle_from: f64, idle_to: f64, now: f64) {
        // ★(RV2-ROLE-1 (b)) 압축 뒤 재는 창은 조용함·몰림 규칙을 쓰지 않는다 — 시작점이 quiescing 해제가 아니라 압축 관측이라
        //   평상 대기열(압축 전에 들어온 회신)을 사이클이 붙잡은 몰림으로 볼 이유가 없다 · 창은 [`CTX_FLOOR_REREAD_SECS`] 로 끝난다.
        if idle_to - idle_from < CTX_FLOOR_IDLE_QUIET_SECS || self.rearmed || !self.settling(now) {
            return;
        }
        let (Some(anchor), Some(seen)) = (self.settle_anchor, self.settle_seen_at) else { return };
        if self.restore_quiet_from.is_none()
            && idle_from >= anchor
            && self.restore_rise_at.is_some_and(|t| t <= idle_to)
        {
            self.restore_quiet_from = Some(idle_from);
            // 이 틈을 끝낸 입력(대기열 배달)이 이미 기록됐으면 곧바로 확정한다.
            self.freeze_restore(now);
        }
        // 복원 끝 바닥이 확정됐으면(복원 뒤 입력이 흐르는 중):
        //  · 사이클이 붙잡았던 대기열 항목(시작점 전에 들어온 것)이 남아 있으면 몰림 중 — 15초 조용함(대기열 막힘)에서만 닫는다.
        //    몰림 전체가 최고치에 든다(몰림만으로 발화 없음 · 몰림 길이와 무관 · ①).
        //  · 다 배달됐으면(★R2NC6-1 · RV-ROLE-CF42-1) **마지막 대기열 배달 뒤에 시작한** ≥2초 조용함(그 배달 턴의 끝)에서 닫는다 —
        //    그 뒤 들어오는 배달(시작점 뒤에 들어온 항목)은 새 작업이라 바닥이 아니라 성장이다. 대기열이 선 바쁜 좌석(틈 2~15초)도
        //    몰림이 끝나면 곧 닫힌다(d9638152 는 15초 조용함만 기다려 창이 300~600초 열린 채 작업을 실었다 · ②).
        let (quiet, after) = match (self.restore_peak.is_some(), self.held_pending()) {
            (true, true) => (CTX_FLOOR_BURST_QUIET_SECS, None),
            (true, false) => (CTX_FLOOR_IDLE_QUIET_SECS, self.queue_at),
            (false, _) => (CTX_FLOOR_IDLE_QUIET_SECS, None),
        };
        if idle_to - idle_from >= quiet
            && after.is_none_or(|d| idle_from >= d)
            && idle_from >= anchor
            && idle_to >= anchor + CTX_FLOOR_SETTLE_MIN_SECS
            && seen >= anchor
            && self.settle_peak.is_some()
        {
            self.settle_closed = true;
            self.settle_closed_at = Some(now);
        }
    }

    /// 관측 1건 — 정착 창 안이면 바닥 후보(최고치)에 싣는다. 모든 보고에서 교차 판정 **전에** 부른다.
    /// 정착 창 밖이면 Claude 자체 압축을 본다([`Self::note_compaction`] — 재무장했으면 Some).
    pub fn observe(&mut self, pct: u8, window: Option<u64>, now: f64) -> Option<CtxRearm> {
        let sw = window.or(self.session_peak_window);
        let prior = self.session_peak.map(|p| rebase_pct(p, self.session_peak_window, sw));
        self.session_peak = Some(prior.map_or(pct, |p| p.max(pct)));
        self.session_peak_window = sw;
        self.last_pct = Some((pct, window));
        if !self.settling(now) {
            return self.note_compaction(pct, window, prior, now);
        }
        let cur = self.floor_for(window);
        let dropped = cur.is_some_and(|f| u16::from(pct) + u16::from(CTX_FLOOR_COMPACT_DROP) <= u16::from(f));
        // ★(RV2-ROLE-1 (a)) 압축 뒤 재는 창 안의 두 번째 Claude 압축(창 최고치보다 낙폭 이상 낮음)도 압축이다 — 종전에는 창 안
        //   관측을 압축으로 보지 않아 압축 전 최고치가 '압축 뒤 바닥'이 됐다. 그 시각에서 다시 잰다(clear 뒤 창은 종전대로 · 최고치).
        if self.rearmed && dropped {
            let b = self.bar(0, window);
            self.restart_measure(pct, window, now);
            return b.map(|b| CtxRearm { floor: b.settled, regime: b.regime });
        }
        // ★(ADV2-R1-1 · RV2-ROLE-1) clear 뒤 창 안의 Claude 압축(창 최고치보다 낙폭 이상 낮음 · 재무장 대상 세션 — 차단기 영역이거나
        //   막대가 차단 상한 위 · 정상·Limited 세션의 수동 /compact 는 종전대로 최고치를 싣는다)은 곧바로 재무장한다 — 창이 끝날
        //   때까지(최대 600초) 기다리면 그동안 자란 몫이 '압축 뒤 바닥'에 든다(재검증자 반례: 압축 뒤 수준 30 이 42 로 재임). 창
        //   최고치가 선제 압축 천장 위였고 복원 턴 도중(시작점 + 최소 창 안 · 복원 끝 전)이나 사이클이 붙잡은 몰림 도중의 압축이면
        //   사이클 자신의 입력이 선제 압축점을 넘긴 것이다 — 그 clear 는 압축점 위로 돌아온다(참으로 가득 참 · 확인 재료). 복원 끝
        //   뒤 작업이 부른 압축은 아니다(clear 로 돌아오는 바닥 = 복원 끝).
        if !self.rearmed && dropped {
            let w = window.or(self.floor_window);
            if let Some(b) = self.bar(0, window).filter(|b| b.regime.tier3() || b.bar > ctx_floor_hard_cap(w)) {
                // 사이클 자신의 입력이 부른 압축 — 창 최고치가 선제 압축 천장 위였다(보고 간격 때문에 압축 직전 표시는 압축점보다
                // 몇 %p 낮을 수 있다 · 천장 아래 높이의 수동 /compact 는 이 판정에서 뺀다).
                let in_restore = self.restore_peak.is_none() && self.settle_anchor.is_some_and(|a| now - a <= CTX_FLOOR_SETTLE_MIN_SECS);
                if cur.is_some_and(|f| f > ctx_floor_ceiling(w)) && (in_restore || self.held_pending() || self.held_seen) {
                    self.window_compacted = true;
                    self.session_tier3 = true;
                }
                return Some(self.rearm_at(b, pct, window, now));
            }
        }
        if cur.is_none_or(|f| pct > f) {
            self.settle_rise_at = Some(now);
        }
        // 복원 턴의 보고 — 시작점 뒤 처음으로 지금까지의 최고치(없으면 0)를 넘는 관측(붙여넣기를 처리한 턴이 보고됐다).
        if self.restore_rise_at.is_none() && pct > cur.unwrap_or(0) && self.settle_anchor.is_some_and(|a| now >= a) {
            self.restore_rise_at = Some(now);
        }
        self.settle_peak = Some(cur.map_or(pct, |f| f.max(pct)));
        self.settle_seen_at = Some(now);
        if !self.rearmed && self.settle_anchor.is_some_and(|a| now >= a && now - a <= CTX_FLOOR_SETTLE_MIN_SECS) {
            self.early_peak = self.settle_peak;
            self.early_peak_window = window.or(self.floor_window);
        }
        // 대기열 인계로 확정한 직후 · 그 배달이 제출(CR)되기 전의 보고는 복원 턴의 늦은 보고다(상태줄이 인계보다 늦게 도착) —
        // 복원 끝 바닥에 싣는다. 제출 뒤 보고는 새 메시지를 담을 수 있다(싣지 않는다).
        if let (Some(rp), Some(fa), Some(until)) = (self.restore_peak, self.restore_freeze_at, self.restore_grace_until) {
            if now <= until && self.inject_done_at.is_none_or(|d| d <= fa) {
                self.restore_peak = Some(rp.max(rebase_pct(pct, window, self.restore_window)));
            }
        }
        if window.is_some() {
            self.floor_window = window;
        }
        None
    }

    /// ★(R2NC5-1 (b)) 정착 창이 닫힌 clear 뒤 세션에서 컨텍스트가 잰 바닥(복원 끝 · 없으면 정착 창 최고치)보다
    /// [`CTX_FLOOR_COMPACT_DROP`] 이상 낮게 보고되면 Claude 자체 압축(자동·수동 /compact — 세션 파일은 그대로)이다. 그 세션이
    /// 차단기 영역(Probe·Stopped)이거나 막대가 차단 상한 위(선제 압축이 cys clear 보다 먼저 오는 높이)였으면 잰 바닥을 버리고
    /// **재무장**한다 — Stopped 는 새 세션이 없으면 다시 재지 않으므로, 이것이 없으면 그 좌석은 오너가 손으로 clear·재기동할
    /// 때까지 cys 사이클(저장·지침 재주입)을 영영 받지 못한다(②). 정상·Limited 세션(막대 ≤ 차단 상한 · 선제 압축점 아래)은
    /// 건드리지 않는다. 창 전환(1M)은 바닥을 토큰 비율로 옮긴 뒤 비교하므로 압축으로 세지 않는다. 낙폭은 **같은 세션** 안이어야
    /// 한다([`Self::session_peak`] — 발화 없이 바뀐 새 세션(잰 바닥을 유지하는 교체)의 낮은 첫 보고는 압축이 아니다 · 그 오판은
    /// 유휴 좌석을 기본 임계로 되돌려 재주입 고리를 한 번 더 부른다 — ①).
    ///
    /// ★(R1-RUNAWAY-1 · ①) 재무장은 기본 임계로 곧장 가지 않고 **압축 뒤 바닥을 다시 잰다** — 압축 관측 시각을 시작점으로 정착
    /// 창을 새로 연다(clear 뒤 세션과 같은 기계: 최고치 · 복원 턴 끝 · 몰림 · 조용함 닫힘 · 뒷문 없음). 압축 뒤 SessionStart:compact
    /// 훅은 지침 전문(MASTER 96.7KB · CEO 105KB)을 다시 읽게 해 압축 뒤 수준이 30 + 30~40%p = 60~70% 다 — d9638152 는 그 수준이
    /// 기본 임계(60) 위면 성장 0 으로 곧바로 발화했고, 그 사이클의 붙여넣기+복원이 선제 압축점(83.5%) 위인 좌석은 붙여넣기 →
    /// 압축 → 재읽기 → 재무장 발화를 주기 신호마다 돌았다(유휴 4시간 19~21회). 이제 압축 뒤 발화는 그 바닥 위로 실제로 자란 뒤
    /// ([`ctx_floor_bar_measured`] — 바닥 + 여유 · 천장·차단 상한)뿐이다: 압축 1회당 사이클 ≤ 1 · 그것도 실제 성장이 부른다.
    /// 다시 잰 바닥도 차단기 높이면 다음 압축이 또 잰다(한 세션 여러 번 — 영구 중단 없음). 확인 재료(`prev_tier3`·`session_tier3`)
    /// 와 뒷문 소진은 그대로 둔다(압축 뒤 바닥은 clear 사이클 자신의 입력이 아니다).
    fn note_compaction(&mut self, pct: u8, window: Option<u64>, session_prior: Option<u8>, now: f64) -> Option<CtxRearm> {
        // 같은 세션 안에서 떨어졌어야 한다 — 마지막 교체 통지(또는 재무장) 뒤 관측 최고치보다도 낙폭 이상 낮다.
        let drop = u16::from(CTX_FLOOR_COMPACT_DROP);
        if session_prior.is_none_or(|p| u16::from(pct) + drop > u16::from(p)) {
            return None;
        }
        let w = window.or(self.floor_window);
        let b = self.bar(0, w)?;
        if !(b.regime.tier3() || b.bar > ctx_floor_hard_cap(w)) {
            return None;
        }
        // 보고 바닥(복원 끝 · 오염이면 정착 창 최고치) 기준 낙폭.
        if u16::from(pct) + drop > u16::from(b.floor) {
            return None;
        }
        Some(self.rearm_at(b, pct, window, now))
    }

    /// 재무장 1건 — clear 로 돌아오는 바닥을 남기고 압축 관측 시각에서 바닥을 다시 잰다.
    fn rearm_at(&mut self, b: CtxFloorBar, pct: u8, window: Option<u64>, now: f64) -> CtxRearm {
        let w = window.or(self.floor_window);
        // ★(ADV2-R1-1 · ①) 압축 전 clear 세션의 clear 로 돌아오는 바닥(정착 창 최고치 — 복원 + clear 마다 되풀이되는 몰림)을
        //   남긴다 — 압축 뒤 발화는 그 + 1 이상에서만. 참으로 가득 찬 좌석(clear 바닥 76~85)에서 압축 뒤 바닥 63~70 위 75·80 발화는
        //   cys clear 가 컨텍스트를 오히려 올리고 그 붙여넣기+복원이 다음 압축을 불러 성장 약 4.5%p 마다 clear → 압축 → 재무장을
        //   돌았다(재검증자 24시간 발화 11 · 압축 10). 두 번째 이후 압축(재무장 세션 안)은 처음 남긴 값을 그대로 쓴다.
        if !self.rearmed {
            // clear 로 돌아오는 바닥: 사이클 자신의 입력이 창 안에서 압축을 불렀으면 선제 압축점 · 사이클이 붙잡은 몰림(복원 턴
            // 동안 선 회신 포함)이 있었거나 복원 턴이 기계 주입으로 오염됐으면 정착 창 최고치(복원 끝 + clear 마다 되풀이되는 몰림) ·
            // 몰림이 없으면 복원 끝(정착 창 최고치는 복원 뒤 첫 작업 턴 — 긴 턴이면 몇 %p — 을 실을 수 있다 · 그 몫까지 들어올리면
            // 압축 뒤 발화가 선제 압축점 턱밑으로 밀려 압축이 cys clear 를 앞지른다) · 복원 끝 신호가 없으면 시작점 + 최소 창까지의
            // 최고치.
            self.rearm_floor = Some(if self.window_compacted {
                ctx_pct_below_reserve(w, CC_SUMMARY_RESERVE_TOKENS + CC_AUTOCOMPACT_BUFFER_TOKENS)
            } else if self.restore_peak.is_some() {
                if self.held_seen || self.restore_tainted { b.settled } else { b.floor }
            } else {
                // 복원 끝 신호가 없고 창이 시간으로 닫혔다(끊김 없는 긴 턴) — 시작점 + 최소 창까지의 최고치(복원 턴 몫)로 추정.
                self.early_peak.map_or(b.settled, |e| rebase_pct(e, self.early_peak_window, w).min(b.settled))
            });
            self.rearm_floor_window = w;
            self.rearm_thin = b.thin || b.regime == CtxFloorRegime::Backlog;
        }
        self.restart_measure(pct, window, now);
        CtxRearm { floor: b.floor, regime: b.regime }
    }

    /// Claude 압축 관측 시각에서 바닥을 다시 재기 시작한다([`CTX_FLOOR_REREAD_SECS`] 창 · 뒷문·복원 턴·몰림 규칙 없음).
    fn restart_measure(&mut self, pct: u8, window: Option<u64>, now: f64) {
        self.rearmed = true;
        // 압축 관측 시각에서 바닥을 다시 잰다 — 시작점·교체 시각을 여기로(사이클 quiescing 이 시작점을 밀 때의 상한도 여기서 센다).
        self.reset_at = Some(now);
        self.settle_anchor = Some(now);
        self.settle_peak = Some(pct);
        if window.is_some() {
            self.floor_window = window;
        }
        self.settle_rise_at = None;
        self.settle_seen_at = Some(now);
        self.settle_closed = false;
        self.restore_rise_at = None;
        self.restore_quiet_from = None;
        self.restore_peak = None;
        self.restore_window = None;
        self.restore_freeze_at = None;
        self.restore_grace_until = None;
        self.session_notice = None;
        self.session_fed = false;
        self.restore_tainted = false;
        self.paste_input_at = None;
        self.backlog_cutoff = None;
        self.settle_closed_at = None;
        self.window_compacted = false;
        self.held_seen = false;
        self.early_peak = None;
        self.early_peak_window = None;
        // '같은 세션' 낙폭의 기준도 압축 뒤부터 — 다음 압축은 다시 잰 바닥 기준으로 본다.
        self.session_peak = Some(pct);
        self.session_peak_window = window.or(self.session_peak_window);
    }

    /// 이 좌석의 실효 임계 — clear 뒤 세션이면 max(기본, 바닥 + 여유)([`ctx_floor_bar`]) · 아니면 기본.
    /// ★정착 창 안 뒷문(② 봉인): 창 안에서는 바닥이 관측을 따라 올라 발화가 없다 — clear 직후 몇 분 만에 실제 작업으로
    /// 차단 상한(200K 80)을 넘는 좌석이 창이 닫힐 때까지(최대 10분) 끌려가지 않도록, 창 안에서도 max(기본, 차단 상한)
    /// 에서는 발화한다. 뒷문은 **연속 두 세션에 걸쳐 쓰지 않는다**([`Self::backstop_spent`]) — 붙여넣기·복원만으로 차단
    /// 상한을 넘는 유휴 좌석은 뒷문 1회 뒤 다음 세션에서 바닥이 증명돼 Stopped 로 멈춘다(①).
    pub fn effective_threshold(&mut self, base: u8, window: Option<u64>, now: f64) -> u8 {
        let Some(CtxFloorBar { bar, regime, .. }) = self.bar(base, window) else { return base };
        // ★(RV2NC-E20-1) 몰림을 바닥으로 확정한 세션(Backlog)도 다음 세션의 확인 재료다 — 일하는 좌석의 빠른 발화만 푼다.
        if regime.tier3() || regime == CtxFloorRegime::Backlog {
            self.session_tier3 = true;
        }
        // 압축 뒤 다시 재는 세션(R1-RUNAWAY-1)의 창에는 뒷문이 없다 — 창 안에서는 바닥이 관측을 따라 올라 발화하지 않는다.
        if self.settling(now) && !self.backstop_spent && !self.rearmed {
            bar.min(base.max(ctx_floor_hard_cap(window.or(self.floor_window))))
        } else {
            bar
        }
    }

    /// 기본 임계는 넘었지만 실효 임계 아래(`pct < 실효`)인 보고 1건의 고지 판정. 이벤트(`context.floor_raised`)는 같은
    /// 세션 안에서 영역이 올라갈 때마다(좌석의 직전 (영역, 임계)와 다를 때만), 오너 feed 는 **정착 창이 닫힌 뒤** 세션당
    /// 1번 — 복원 도중의 바닥(예: 72 → 78)으로 오너에게 말하지 않도록 잰 바닥이 정해진 뒤의 영역으로 센다.
    pub fn hold_notice(&mut self, pct: u8, base: u8, window: Option<u64>, now: f64) -> Option<CtxHoldNotice> {
        if pct < base {
            return None;
        }
        let CtxFloorBar { bar, regime, floor, settled, tainted, lift, .. } = self.bar(base, window)?;
        if pct >= bar {
            return None;
        }
        let escalated = self.session_notice.is_none_or(|r| regime > r);
        if escalated {
            self.session_notice = Some(regime);
        }
        let event = escalated && self.last_announced != Some((regime, bar));
        if event {
            self.last_announced = Some((regime, bar));
        }
        let pow2 = |n: u32| n.is_power_of_two();
        let mut count = 0;
        let mut feed = None;
        if !self.settling(now) && !std::mem::replace(&mut self.session_fed, true) {
            (feed, count) = match regime {
                CtxFloorRegime::Raise => (if std::mem::replace(&mut self.raise_fed, true) { None } else { Some("warn") }, 1),
                CtxFloorRegime::Limited | CtxFloorRegime::Backlog | CtxFloorRegime::Probe => {
                    self.limited_count += 1;
                    (pow2(self.limited_count).then_some("warn"), self.limited_count)
                }
                CtxFloorRegime::Stopped => {
                    self.stopped_count += 1;
                    (pow2(self.stopped_count).then_some("error"), self.stopped_count)
                }
            };
        }
        if !event && feed.is_none() {
            return None;
        }
        let w = window.or(self.floor_window);
        Some(CtxHoldNotice {
            floor,
            settled,
            bar,
            regime,
            ceiling: ctx_floor_ceiling(w),
            hard_cap: ctx_floor_hard_cap(w),
            event,
            feed,
            count,
            after_compaction: self.rearmed,
            tainted,
            lift,
        })
    }

    /// 래치가 소진된 교차(`pct >= 실효 임계`) 1건 — 언제나 발화다(보류는 실효 임계가 이미 했다). 발화 시각을 기록한다.
    /// 정착 창 안의 발화는 뒷문뿐이다(창 안 바닥 ≥ pct 라 막대는 pct 위) — 이 세션을 차단 상한 근처로 세고(다음 세션
    /// 확인 재료) 뒷문을 쓴 것으로 적는다. 창 밖의 **정상·Limited** 발화(바닥이 차단 상한에서 먼 좌석의 실제 성장)만
    /// 뒷문을 다시 무장한다 — Probe·Stopped 좌석에 뒷문을 다시 주면 창 안 1~2%p 에 지침을 또 붙여 넣는 값싼 사이클이
    /// Stopped 사이클과 번갈아 돈다(①).
    ///
    /// ★(ROLE-R4-1 · R2NC5-1) 복원 끝 바닥이 확정됐고 여유가 있으면(정상·Limited 높이) 창 안 뒷문 발화는 복원 뒤 작업(사이클이
    /// 붙잡았던 배달)이 부른 것이라 이 세션을 차단 상한 근처로 세지 않는다. 뒷문을 쓴 것은 그대로 적고, 재무장도 종전처럼
    /// 정착 창 최고치의 영역으로 판정한다 — 복원 끝 판정이 틀려도 뒷문이 연속 세션에 걸쳐 돌지 않는다(①).
    pub fn on_crossing(&mut self, base: u8, window: Option<u64>, now: f64) -> CtxFire {
        let info = self.bar(base, window);
        // 압축 뒤 다시 잰 세션(R1-RUNAWAY-1)에는 뒷문이 없다 — 그 바닥은 clear 사이클 자신의 입력(붙여넣기·복원)이 아니다.
        let settle_backstop = self.settling(now) && !self.rearmed;
        if settle_backstop {
            if !self.restore_roomy(window) {
                self.session_tier3 = true;
            }
            self.backstop_spent = true;
        } else if !self.rearmed
            && (self.settled_regime(window).is_some_and(|r| matches!(r, CtxFloorRegime::Raise | CtxFloorRegime::Limited))
                || (self.restore_roomy(window)
                    && info.is_some_and(|b| b.settled < base.max(ctx_floor_hard_cap(window.or(self.floor_window))))))
        {
            // ★(R2NC6-1) 복원 끝에 여유가 있고 막대가 차단 상한 이하(정착 창 최고치 + 1 ≤ 차단 상한 — 몰림이 여유를 넘지 않았다)인
            //   세션의 창 밖 발화도 다시 무장한다: 정착 창 최고치가 76~79 라 영역이 Probe 로 보여도 복원 끝은 차단기 재료가 아니다
            //   (d9638152 는 그런 좌석의 뒷문을 첫 세션 뒤 영영 다시 주지 않았다). 최고치가 차단 상한 이상(몰림 ≥ 여유 · 막대 =
            //   최고치 + 1)이면 다시 주지 않는다 — 몰림만으로 뒷문이 도는 세션과 성장 1%p 발화가 번갈아 도는 고리 없음(①).
            self.backstop_spent = false;
        }
        // ★(RV2NC-E20-1 · ①·② 빈도 상한) 얇은 세션(또는 얇은 세션에서 압축으로 재무장한 세션)의 창 밖 발화는 성장 속도로 가른다 —
        //   일하는 좌석(분당 0.2%p 이상 · 반올림 뺀 하한)이면 확인 재료에서 빼고(다음 세션도 차단 상한 근처에서 clear · ②), 느리면
        //   (주기 신호·유휴) 이 세션을 확인 재료로 적는다(다음 세션은 몰림 위 최소 여유 성장마다 · ①).
        let thin = info.is_some_and(|b| b.thin || b.regime == CtxFloorRegime::Backlog) || (self.rearmed && self.rearm_thin);
        if thin && !settle_backstop && self.post_clear() {
            if self.thin_growth_fast(window, now) {
                self.session_fast = true;
            } else {
                self.session_tier3 = true;
                self.session_fast = false;
            }
        }
        let after_compaction = self.post_clear() && self.rearmed;
        self.fired_at = Some(now);
        CtxFire {
            floor: info.map(|b| b.floor),
            regime: info.map(|b| b.regime),
            settle_backstop,
            settled: info.map(|b| b.settled),
            after_compaction,
        }
    }
}

/// 좌석 출력의 조용함 관측(데몬 단조 초) — [`CtxLoopGuard::note_idle`] 재료. `quiet_since` = 마지막 출력 시각(그때부터
/// 지금까지 조용하다) · `last_gap` = 마지막으로 **끝난** 조용한 틈(길이 [`CTX_FLOOR_IDLE_QUIET_SECS`] 이상 — PTY reader 기록) ·
/// `last_input` = 좌석에 마지막으로 입력이 쓰인 시각([`CtxLoopGuard::note_input`] 재료) · `last_queue_delivery` = 마지막
/// 대기열 배달 인계 시각(`Surface::last_queue_delivery_at`) · `inject_done` = writer 의 마지막 Inject 끝(CR 기록 ·
/// `InjectTrack::done_at`) — 둘 다 [`CtxLoopGuard::note_delivery`] 재료 · `queue_oldest` = 좌석 대기열(`Surface::pending_queue`)에
/// 남은 가장 오래된 항목의 enqueue 시각(없으면 빈 대기열 — [`CtxLoopGuard::note_queue`] 재료).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CtxIdleObs {
    pub quiet_since: f64,
    pub last_gap: Option<(f64, f64)>,
    pub last_input: Option<f64>,
    pub last_queue_delivery: Option<f64>,
    pub inject_done: Option<f64>,
    pub queue_oldest: Option<f64>,
}

impl CtxIdleObs {
    /// 조용함 신호 없음(출력이 끊이지 않는 좌석과 같다) — 정착 창은 종전 시간 창대로.
    #[cfg(test)]
    pub const NONE: CtxIdleObs = CtxIdleObs {
        quiet_since: f64::INFINITY,
        last_gap: None,
        last_input: None,
        last_queue_delivery: None,
        inject_done: None,
        queue_oldest: None,
    };

    /// 좌석의 지금 관측 — 락은 하나씩 잡고 놓는다(출력 시각 → 틈 → 입력 시각 셋 → 대기열 · 전부 말단).
    pub fn of(daemon: &Daemon, s: &Surface) -> Self {
        let mono = |t: std::time::Instant| t.saturating_duration_since(daemon.started_instant).as_secs_f64();
        let quiet_since = mono(*s.last_output.lock().unwrap_or_else(|e| e.into_inner()));
        let last_gap = s.last_output_gap.lock().unwrap_or_else(|e| e.into_inner()).map(|(a, b)| (mono(a), mono(b)));
        let last_queue_delivery = s.last_queue_delivery_at.lock().unwrap_or_else(|e| e.into_inner()).map(mono);
        let inject_done = s.inject_track.done_at().map(mono);
        // enqueue 시각은 벽시계(epoch 초)다 — 지금의 단조 초에서 경과만큼 뺀다(시계 조정은 몇 초 · 몰림 판정에 무해).
        let oldest_epoch = s
            .pending_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|e| e.enqueued_at)
            .filter(|t| t.is_finite())
            .reduce(f64::min);
        let (now_mono, now_epoch) = (daemon.started_instant.elapsed().as_secs_f64(), crate::state::now_epoch());
        let queue_oldest = oldest_epoch.map(|t| now_mono - (now_epoch - t).max(0.0));
        CtxIdleObs { quiet_since, last_gap, last_input: last_input_of(s).map(mono), last_queue_delivery, inject_done, queue_oldest }
    }
}

/// ★(ROLE-R4-1 · R2NC5-1) 좌석에 마지막으로 **입력이 쓰인** 시각 — 세 기록의 최댓값(결측은 값이 아니다 — 셋 다 없으면 None):
/// writer 의 데몬 Inject 끝(`InjectTrack::done_at` — 대기열·채널·스케줄·CEO 배달·사이클 재주입 전부가 이 arm 을 지난다) ·
/// 직접 기계 send(`last_injected` — `cys send` 본문) · 사람 입력(`last_human_input`). 직접 `send-key` 단독(미리 친 본문의
/// Return)은 기록이 없다 — 그 제출은 입력으로 보이지 않아 복원 끝 바닥이 확정되지 않는다(정착 창 최고치 = 종전 거동 쪽).
fn last_input_of(s: &Surface) -> Option<std::time::Instant> {
    let inject = s.inject_track.done_at();
    let machine = *s.last_injected.lock().unwrap_or_else(|e| e.into_inner());
    let human = *s.last_human_input.lock().unwrap_or_else(|e| e.into_inner());
    [inject, machine, human].into_iter().flatten().max()
}

/// ★(RV2NC-E20-2) 좌석이 clear 사이클의 복원 턴 도중인가([`CtxLoopGuard::restore_turn_open`]) — 채널 inbox·스케줄 직접 push 의
/// 보류 판정. 가드 락은 말단이다(호출부는 다른 락을 쥐지 않은 채 부른다).
pub(crate) fn restore_turn_open(daemon: &Daemon, s: &Surface) -> bool {
    let now = daemon.started_instant.elapsed().as_secs_f64();
    s.ctx_loop_guard.lock().unwrap_or_else(|e| e.into_inner()).restore_turn_open(now)
}

/// ★(clear 직후 바닥 가드) 수집기 틱 — 보고가 없는 동안(유휴 좌석 · 대기열 틈)에도 사이클 진행(quiescing)으로 정착 창
/// 시작점을 밀고, 좌석이 조용해지면(복원 턴 끝) 정착 창을 닫는다. 락은 하나씩(상태 → 출력 시각·틈 → 가드 · 가드는 말단).
fn ctx_guard_tick(daemon: &Daemon, s: &Surface) {
    let now = daemon.started_instant.elapsed().as_secs_f64();
    let quiescing = s
        .agent_status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|st| st.state == "quiescing");
    ctx_guard_tick_at(s, now, quiescing, CtxIdleObs::of(daemon, s));
}

/// [`ctx_guard_tick`] 의 본체(시각·관측을 받는다 — 검체가 가짜 시각으로 수집기 틱을 흉내 낸다).
pub(crate) fn ctx_guard_tick_at(s: &Surface, now: f64, quiescing: bool, idle: CtxIdleObs) {
    let mut g = s.ctx_loop_guard.lock().unwrap_or_else(|e| e.into_inner());
    if quiescing {
        g.note_quiescing(now);
    }
    g.note_input(idle.last_input, now);
    g.note_delivery(idle.last_queue_delivery, idle.inject_done, now);
    g.note_queue(idle.queue_oldest);
    if let Some((from, to)) = idle.last_gap {
        g.note_idle(from, to, now);
    }
    g.note_idle(idle.quiet_since, now, now);
}

/// 두 관측의 세션 파일이 **다른 세션**인가 — 둘 다 비어 있지 않고 파일 줄기(세션 id)가 다를 때만 참.
/// 경로 표기 차이(심링크·/private 접두)는 줄기가 같아 교체로 세지 않는다. 결측은 교체가 아니다(가드 비발동 = 종전 거동).
pub(crate) fn session_file_changed(prev: &str, next: &str) -> bool {
    if prev.trim().is_empty() || next.trim().is_empty() {
        return false;
    }
    let stem = |p: &str| std::path::Path::new(p).file_stem().map(|s| s.to_os_string());
    match (stem(prev), stem(next)) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
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
    // 관측(위) 뒤에 — 같은 틱에 읽힌 복원 턴의 마지막 관측이 조용함으로 창이 닫히기 전에 실린다.
    for s in surfaces.iter().filter(|s| !s.exited.load(Ordering::Relaxed)) {
        ctx_guard_tick(daemon, s);
    }
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
        let old_path = tails.get(&s.id).map(|t| t.path.to_string_lossy().into_owned());
        tails.insert(s.id, TailState::attach(daemon, path.clone(), heuristic, now));
        // 새 세션 파일 = 새 세션 — 에지 게이트 재무장. 직전 세션이 임계 위에서 끝났어도
        // 새 세션이 곧장 임계 이상으로 시작하면(거대 지침 재주입) 발화해야 한다.
        s.ctx_threshold_armed.store(true, Ordering::Relaxed);
        // ★(RV-R2NC-1) 같은 교체를 clear 직후 바닥 가드에도 알린다 — 그 세션이 **바닥만으로** 임계를 넘으면
        //   재발화 대신 임계를 올린다(`CtxLoopGuard`). 첫 부착(이전 tail 없음)은 교체 증거가 아니다(가드 비발동).
        if old_path.is_some_and(|o| session_file_changed(&o, &path.to_string_lossy())) {
            s.ctx_loop_guard
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .note_session_change(daemon.started_instant.elapsed().as_secs_f64());
        }
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
    if cfg!(windows) {
        return None; // lsof 부재(호출부와 같은 이유 · 스폰 0 — 콘솔 창 정책 대상 밖)
    }
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

/// ★(0.14.42 · RV-R2NC-1 → RR3-R1-1 · G3ROLE-1) clear 직후 바닥 가드의 순수 판정 핀 — 시각은 단조 초를 직접 준다.
/// 통합 핀은 `handlers::tests::context_threshold_*floor*` · `context_threshold_floor_guard_*`.
#[cfg(test)]
mod ctx_loop_guard_tests {
    use super::*;

    const W200K: Option<u64> = Some(200_000);
    const W1M: Option<u64> = Some(1_000_000);
    const WINDOWS: [Option<u64>; 3] = [W200K, W1M, None];
    /// 부트·첫 교차처럼 clear 뒤 세션이 아닌 발화.
    const FIRE: Option<CtxFire> =
        Some(CtxFire { floor: None, regime: None, settle_backstop: false, settled: None, after_compaction: false });

    /// 게이트(`handlers::maybe_fire_context_threshold_at`)와 같은 순서: 관측 → 실효 임계 → (미만: 재무장 + 보류 고지 ·
    /// 이상: 래치 소진 → 발화).
    fn report(g: &mut CtxLoopGuard, armed: &mut bool, pct: u8, base: u8, window: Option<u64>, now: f64) -> Option<CtxFire> {
        g.observe(pct, window, now);
        let eff = g.effective_threshold(base, window, now);
        if pct < eff {
            *armed = true;
            let _ = g.hold_notice(pct, base, window, now);
            return None;
        }
        if !std::mem::replace(armed, false) {
            return None;
        }
        Some(g.on_crossing(base, window, now))
    }

    fn post(floor: u8, regime: CtxFloorRegime) -> Option<CtxFire> {
        Some(CtxFire { floor: Some(floor), regime: Some(regime), settle_backstop: false, settled: Some(floor), after_compaction: false })
    }

    /// 복원 끝 바닥(`floor`)이 차단기를 푼 발화 — 정착 창 최고치 `settled` 는 복원 뒤 작업을 담았다.
    fn post_split(floor: u8, settled: u8, regime: CtxFloorRegime) -> Option<CtxFire> {
        Some(CtxFire { floor: Some(floor), regime: Some(regime), settle_backstop: false, settled: Some(settled), after_compaction: false })
    }

    fn backstop(floor: u8, regime: CtxFloorRegime) -> Option<CtxFire> {
        Some(CtxFire { floor: Some(floor), regime: Some(regime), settle_backstop: true, settled: Some(floor), after_compaction: false })
    }

    /// clear(새 세션) — 수집기처럼 래치도 재무장한다(고리에 가장 불리한 쪽).
    fn clear(g: &mut CtxLoopGuard, armed: &mut bool, now: f64) {
        g.note_session_change(now);
        *armed = true;
    }

    /// 상태줄 `used_percentage` 는 반올림이다 — p% 로 보이는 최대 토큰(직전)은 (p+0.5)% 다. 사이클 여유는 거기서 잰다.
    fn max_tokens_shown_as(pct: u8, window: u64) -> u64 {
        (2 * pct as u64 + 1) * window / 200
    }

    // ───────────── 좌석 모형(CSO 가 발화마다 사이클을 돈다) ─────────────

    /// 대기열 배달 전 좌석 조용함(초) — `CYS_QUEUE_QUIET_SECS` 기본. 사이클이 붙잡았던 배달은 복원 턴이 끝나고 이만큼 뒤에 온다.
    const QUEUE_QUIET: f64 = 3.0;

    /// 좌석 한 대의 시나리오. 시각은 초 · 컨텍스트는 %(보고 때 반올림).
    #[derive(Clone, Copy, Debug)]
    struct Seat {
        base: u8,
        window: Option<u64>,
        /// clear 뒤 지침 붙여넣기 직후 컨텍스트.
        paste: f64,
        /// 붙여넣기 뒤 복원 성장(%p)과 그 시간(초).
        restore: f64,
        restore_secs: f64,
        /// 복원이 끝난 뒤 작업 시작까지 쉬는 시간(초) · 그 뒤 분당 성장(%p · 0 = 유휴).
        work_after: f64,
        work_per_min: f64,
        /// 작업 구간의 매 분 가운데 턴(출력 중)인 초 — 나머지는 조용하다(대기열 배달 사이 · 60 = 끊김 없는 한 턴).
        /// 성장은 턴 안에서만 일어난다.
        turn_secs: f64,
        /// 발화 → CSO 수신·저장 지시·검증까지(초) · clear 뒤 SessionStart·붙여넣기까지 quiescing(초).
        cycle_delay: f64,
        quiesce: f64,
        /// 틱(초 · 종전 모형 5) — 대기열 조용함(3초)을 보려면 1초.
        tick: f64,
        /// ★(ROLE-R4-1 · R2NC5-1) 사이클(quiescing·복원) 동안 쌓여 **복원 턴 뒤** 대기열로 배달되는 몫(%p) · 배달 턴 수 · 한
        /// 턴 길이(초) — 배달마다 좌석 조용함 [`QUEUE_QUIET`] 뒤 한 턴(좌석 입력). 작업은 그 뒤 `work_after` 부터.
        backlog: f64,
        backlog_turns: u32,
        backlog_turn_secs: f64,
        /// 운영 신호 모형 — 좌석 입력 시각을 기록하고(붙여넣기·배달·작업 턴 시작 = writer Inject·send 기록) 턴이 끝날 때마다
        /// 보고한다(상태줄). 끄면 종전 모형(입력 신호 없음 · 30초 보고 — 복원 끝 바닥이 확정되지 않는다 = 종전 판정).
        real: bool,
        /// 복원 턴 가운데 출력이 멎는 틈(턴 시작부터 초, 길이 초) — 입력 없음(같은 턴이 이어진다 · 복원 끝 판정의 반례).
        stall: Option<(f64, f64)>,
        /// 턴 끝 → 다음 대기열 배달 사이(초) — 마커 없는 좌석은 조용함 3초 · claude 좌석의 프롬프트 경계 배달은 조용함을 기다리지
        /// 않는다(샌드박스 실측 0.1~0.5초 — 2초 틈이 없다).
        queue_gap: f64,
        /// 복원 턴 가운데(턴 시작부터 초) 바쁨을 보지 않는 기계 입력(채널 행 · 스케줄 push — 대기열 배달 아님) 1건.
        mid_input: Option<f64>,
        /// Claude 자체 선제 압축점(%) — 제출(좌석 입력) 때 컨텍스트가 이 이상이면 같은 세션에서 `compact_to` 로 먼저 압축한다.
        compact_at: Option<f64>,
        compact_to: f64,
        /// ★(R1-RUNAWAY-1) Claude 압축 뒤 SessionStart:compact 훅이 시키는 지침 전문 재읽기(%p) — 압축 직후 [`REREAD_SECS`]
        /// 동안 한 턴(출력 중 · 좌석 입력 없음)으로 오른다. 압축 뒤 수준 = `compact_to` + 이것(MASTER 96.7KB·CEO 105KB ≈ 32~39%p).
        /// 0 = 종전 모형(압축 뒤 `compact_to` 에 머문다).
        reread: f64,
        /// ★(RV2NC-E20-1) 사이클 몰림을 **복원된 좌석이 부른 회신**으로 — 복원 턴 끝 이 초 뒤 대기열에 한꺼번에 든다(시작점 뒤 ·
        /// quiescing 이 붙잡은 것이 아니다 · 드릴 rv-d1p). None = 종전(quiescing 동안 enqueue — clear 1초 뒤).
        backlog_reply: Option<f64>,
        /// ★(RV2NC-E20-2) 복원 턴 도중(붙여넣기부터 초) 바쁨을 보지 않는 기계 주입(채널 행 · writer Inject — 대기열 배달 아님) 1건
        /// (주입 시각 오프셋, 성장 %p, 턴 초) — Claude 가 복원 턴 끝에 조용함 없이 곧바로 이어 처리한다(드릴 rv-d3).
        chan: Option<(f64, f64, f64)>,
        /// ★(RV2-ROLE-1) 작업 구간의 매 분 좌석 입력(대기열 배달) — false 면 입력 없이 이어지는 긴 에이전트 턴(도구 연쇄).
        work_inputs: bool,
        /// ★(RV2-ROLE-1) Claude 선제 압축을 제출 때만이 아니라 턴 도중(도구 호출 사이)에도 본다.
        compact_midturn: bool,
    }

    /// 압축 뒤 지침 재읽기 턴의 길이(초).
    const REREAD_SECS: f64 = 20.0;

    impl Seat {
        fn idle(base: u8, window: Option<u64>, paste: f64, restore: f64) -> Self {
            Seat { base, window, paste, restore, restore_secs: 40.0, work_after: 0.0, work_per_min: 0.0, turn_secs: 60.0,
                   cycle_delay: 60.0, quiesce: 15.0, tick: 5.0, backlog: 0.0, backlog_turns: 0, backlog_turn_secs: 10.0,
                   real: false, stall: None, queue_gap: QUEUE_QUIET, mid_input: None, compact_at: None, compact_to: 30.0,
                   reread: 0.0, backlog_reply: None, chan: None, work_inputs: true, compact_midturn: false }
        }

        /// 운영 신호 모형(1초 틱 · 입력 기록 · 턴 끝 보고 · Claude 선제 압축점 = 창 − 33000 토큰 · 창 미상은 200K 로 압축).
        fn real(base: u8, window: Option<u64>, paste: f64, restore: f64) -> Self {
            let w = window.unwrap_or(200_000) as f64;
            let compact = (w - (CC_SUMMARY_RESERVE_TOKENS + CC_AUTOCOMPACT_BUFFER_TOKENS) as f64) * 100.0 / w;
            Seat { tick: 1.0, real: true, compact_at: Some(compact), ..Seat::idle(base, window, paste, restore) }
        }
    }

    #[derive(Debug, Default)]
    struct Run {
        /// (시각, 보고 pct, 판정, 그 세션의 복원 뒤 컨텍스트)
        fires: Vec<(f64, u8, CtxFire, f64)>,
        max_pct: u8,
        /// 첫 clear 뒤(부트 교차 제외) 보고한 최고치.
        max_post_clear: u8,
        /// Claude 자체 압축 시각 · 가드의 재무장 수(R2NC5-1 (b)).
        compactions: Vec<f64>,
        rearms: usize,
    }

    /// 부트(임계 위)에서 첫 교차 발화 → 발화마다 `cycle_delay` 뒤 clear(quiescing `quiesce` 초 · 3%) → 붙여넣기 →
    /// 복원 성장(한 턴) → (사이클 동안 쌓인 대기열 배달 턴들) → `work_after` 쉬고 분당 `work_per_min` 성장(매 분 `turn_secs` 턴
    /// 안에서만 · 100 캡). `tick` 초 틱 · 보고는 30초마다 + 사건 때(+ `real` 이면 턴 끝마다). 운영 두 경로와 같은 순서:
    /// quiescing → 좌석 입력 시각 → 끝난 조용한 틈(보고 **전** — 틈 뒤 보고는 새 턴의 것) → 보고 → 지금 이어지는 조용함(보고
    /// **뒤** — 조용한 동안의 보고는 직전 턴의 끝이다) — 수집기 틱과 같다. `compact_at` 이면 제출 때 Claude 가 먼저 압축한다.
    fn run_seat(seat: Seat, secs: f64) -> Run {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        let mut run = Run::default();
        let boot = f64::from(seat.base.max(60).saturating_add(10).min(100));
        let mut pct = boot;
        let mut pending: Option<f64> = None; // 사이클 clear 예정 시각
        let mut session: Option<f64> = None; // 이 세션의 clear 시각
        let mut idle_since: Option<f64> = None; // 출력이 조용해진 시각(틱 해상도)
        let mut last_report = -1e9;
        let mut last_input: Option<f64> = None;
        let mut last_queue: Option<f64> = None;
        let (mut last_cr, mut pending_cr): (Option<f64>, Option<f64>) = (None, None);
        let mut chan_inject: Option<f64> = None; // 채널 행 주입의 CR(writer Inject 끝)
        let mut dropped = 0.0; // Claude 압축이 이 세션에서 내린 몫(%p)
        let mut reread_from: Option<f64> = None; // 마지막 Claude 압축(= 지침 재읽기 턴 시작) 시각
        // 지금까지 재읽은 몫(%p) — 압축마다 새로 0 에서 오른다.
        let reread_at = |from: Option<f64>, t: f64| from.map_or(0.0, |a| seat.reread * ((t - a) / REREAD_SECS).clamp(0.0, 1.0));
        let (mut was_busy, mut prev_t) = (false, f64::NEG_INFINITY);
        let mut t = 0.0;
        // 작업 구간(시작부터 dt 초)의 (성장 %p, 턴 중인가) — 매 분 turn_secs 동안만 일하고 자란다.
        let work = |dt: f64| -> (f64, bool) {
            if dt < 0.0 || seat.work_per_min <= 0.0 {
                return (0.0, false);
            }
            let (m, ph) = ((dt / 60.0).floor(), dt % 60.0);
            (seat.work_per_min * (m + (ph / seat.turn_secs).min(1.0)), ph < seat.turn_secs)
        };
        // 작업 턴 시작(좌석 입력 = 대기열 배달)이 구간 (a, b] 안에 있으면 그 시각(가장 늦은 것).
        let work_input = |from: f64, a: f64, b: f64| -> Option<f64> {
            if seat.work_per_min <= 0.0 || b < from || !seat.work_inputs {
                return None;
            }
            let at = from + 60.0 * ((b - from) / 60.0).floor();
            (at > a).then_some(at)
        };
        let restored = (seat.paste + seat.restore).min(100.0);
        let n = seat.backlog_turns;
        let per = if n > 0 { seat.backlog / f64::from(n) } else { 0.0 };
        while t <= secs {
            let mut force = t == 0.0;
            if pending.is_some_and(|c| t >= c) {
                pending = None;
                session = Some(t);
                clear(&mut g, &mut armed, t);
                force = true;
                dropped = 0.0;
                reread_from = None;
            }
            let quiescing = session.is_some_and(|s| t < s + seat.quiesce);
            // 이 틱의 (원시 컨텍스트 — 없으면 그대로, 출력 중, 구간 (prev_t, t] 안의 좌석 입력 · 그 가운데 대기열 배달)
            let (raw, busy, input): (Option<f64>, bool, Option<f64>);
            let mut queued: Option<f64> = None;
            // 대기열에 남은 가장 오래된 항목 — 사이클이 붙잡았던 배달(quiescing 동안 enqueue · clear 1초 뒤)이 아직 남았으면 그
            // 시각. 작업 턴의 배달은 들어오자마자 인계된다(대기열에 남지 않는다).
            let mut queue_oldest: Option<f64> = None;
            if let Some(s) = session {
                let paste_at = s + seat.quiesce;
                let restore_end = paste_at + seat.restore_secs;
                // 채널 행 턴 — 복원 턴 끝(주입이 그 뒤면 주입 시각)에 조용함 없이 곧바로 이어진다.
                let chan_turn = seat.chan.map(|(o, pp, secs)| {
                    let st = restore_end.max(paste_at + o);
                    (paste_at + o, st, st + secs, pp)
                });
                let after_restore = chan_turn.map_or(restore_end, |c| c.2);
                // 회신 몰림(시작점 뒤 enqueue)이면 그 시각 뒤에 배달된다.
                let enq = seat.backlog_reply.map(|d| restore_end + d);
                let first = after_restore.max(enq.unwrap_or(after_restore));
                let start = |k: u32| first + seat.queue_gap + f64::from(k) * (seat.backlog_turn_secs + seat.queue_gap);
                if (0..n).any(|k| start(k) > t) {
                    queue_oldest = match enq {
                        None => Some(s + 1.0),
                        Some(e) => (t >= e).then_some(e),
                    };
                }
                let backlog_end = if n > 0 { start(n - 1) + seat.backlog_turn_secs } else { after_restore };
                let work_from = backlog_end + seat.work_after;
                let (grown, turn) = work(t - work_from);
                let mut inp = (paste_at > prev_t && paste_at <= t).then_some(paste_at);
                let mut delivered = 0.0;
                let mut in_backlog_turn = false;
                for k in 0..n {
                    let (st, en) = (start(k), start(k) + seat.backlog_turn_secs);
                    if t >= en {
                        delivered += per;
                    } else if t >= st {
                        delivered += per * (t - st) / seat.backlog_turn_secs;
                        in_backlog_turn = true;
                    }
                    if st > prev_t && st <= t {
                        inp = Some(st);
                        queued = Some(st);
                    }
                }
                if let Some(w) = work_input(work_from, prev_t, t) {
                    inp = Some(w);
                    queued = Some(w);
                }
                if let Some(m) = seat.mid_input.map(|o| paste_at + o).filter(|m| *m > prev_t && *m <= t) {
                    inp = Some(inp.map_or(m, |i: f64| i.max(m)));
                }
                let (mut chan_grown, mut in_chan_turn) = (0.0, false);
                if let Some((at, st, en, pp)) = chan_turn {
                    if at > prev_t && at <= t {
                        inp = Some(inp.map_or(at, |i: f64| i.max(at)));
                        chan_inject = Some(at + 0.4); // writer Inject 끝(CR) — 대기열 배달이 아니다
                    }
                    chan_grown = pp * ((t - st) / (en - st)).clamp(0.0, 1.0);
                    in_chan_turn = t >= st && t < en;
                }
                input = inp;
                if t < paste_at {
                    (raw, busy) = (Some(3.0), quiescing);
                } else if t < restore_end {
                    let stalled = seat.stall.is_some_and(|(o, d)| t >= paste_at + o && t < paste_at + o + d);
                    (raw, busy) = (Some(seat.paste + seat.restore * (t - paste_at) / seat.restore_secs), !stalled);
                } else {
                    (raw, busy) = (Some(restored + chan_grown + delivered + grown), in_backlog_turn || in_chan_turn || turn);
                }
            } else {
                let (grown, turn) = work(t);
                busy = turn || pending.is_some();
                raw = pending.is_none().then_some(boot + grown);
                input = if pending.is_none() { work_input(0.0, prev_t, t) } else { None };
                queued = input;
            }
            if let Some(i) = input {
                if seat.real {
                    last_input = Some(i);
                    last_queue = queued.or(last_queue);
                    // writer Inject: 붙여넣기 → cr_delay(0.4초) → CR — 제출은 다음 틱에 보인다.
                    pending_cr = queued.map(|q| q + 0.4).or(chan_inject.take()).or(pending_cr);
                }
                // Claude 자체 선제 압축 — 제출 때 컨텍스트가 압축점 이상이면 같은 세션에서 먼저 압축한다(세션 파일 그대로).
                if let (Some(c), Some(r)) = (seat.compact_at, raw) {
                    if r - dropped + reread_at(reread_from, t) >= c {
                        dropped = r - seat.compact_to;
                        reread_from = Some(t);
                        run.compactions.push(t);
                        force = true;
                    }
                }
            }
            // ★(RV2-ROLE-1) 턴 도중(도구 호출 사이)의 Claude 선제 압축.
            if seat.compact_midturn && input.is_none() && busy {
                if let (Some(c), Some(r)) = (seat.compact_at, raw) {
                    if r - dropped + reread_at(reread_from, t) >= c {
                        dropped = r - seat.compact_to;
                        reread_from = Some(t);
                        run.compactions.push(t);
                        force = true;
                    }
                }
            }
            if let Some(r) = raw {
                pct = (r - dropped + reread_at(reread_from, t)).clamp(0.0, 100.0);
            }
            // 압축 뒤 지침 재읽기 턴 — 출력 중(입력 없음).
            let busy = busy || (seat.reread > 0.0 && reread_from.is_some_and(|a| t < a + REREAD_SECS));
            if quiescing {
                g.note_quiescing(t);
            }
            if seat.real {
                if pending_cr.is_some_and(|c| c <= t) {
                    last_cr = pending_cr.take();
                }
                g.note_input(last_input, t);
                g.note_delivery(last_queue, last_cr, t);
                g.note_queue(queue_oldest);
            }
            if busy {
                if let Some(s) = idle_since.take() {
                    g.note_idle(s, t, t); // 끝난 틈 — 이 틱의 보고(새 턴)보다 먼저
                }
            } else if idle_since.is_none() {
                idle_since = Some(t);
            }
            let turn_end = seat.real && was_busy && !busy;
            if force || turn_end || t - last_report >= 30.0 {
                last_report = t;
                let p = pct.round() as u8;
                run.max_pct = run.max_pct.max(p);
                if session.is_some() {
                    run.max_post_clear = run.max_post_clear.max(p);
                }
                let rearmed = g.rearmed;
                if let Some(f) = report(&mut g, &mut armed, p, seat.base, seat.window, t) {
                    run.fires.push((t, p, f, restored));
                    pending = Some(t + seat.cycle_delay);
                }
                if g.rearmed && !rearmed {
                    run.rearms += 1;
                }
            }
            if let Some(s) = idle_since {
                g.note_idle(s, t, t); // 이어지는 조용함 — 수집기 틱
            }
            was_busy = busy;
            prev_t = t;
            t += seat.tick;
        }
        run
    }

    // ───────────── 천장·상한 유도 ─────────────

    /// ★(R2NC3-1) 천장은 창에서 유도한다 — 창 − (요약 출력 예약 20000 + 자동 압축 버퍼 13000 + 사이클 여유 15000) 를
    /// 상태줄 반올림까지 넣어 퍼센트로 내림, 85 캡. 모든 창에서 (천장으로 보이는 최대 토큰) + 사이클 여유 ≤ Claude Code
    /// 선제 압축점 < 차단점. 창을 모르면 claude 최소 창(200K)으로 본다(천장이 낮아지는 쪽 = 발화 쪽 실패).
    #[test]
    fn ctx_floor_ceiling_is_derived_from_the_window_below_claude_code_limits() {
        assert_eq!(ctx_floor_ceiling(W200K), 75, "200K 창 천장");
        assert_eq!(ctx_floor_ceiling(W1M), CTX_FLOOR_CEIL, "1M 창은 종전 85 캡");
        assert_eq!(ctx_floor_ceiling(None), ctx_floor_ceiling(W200K), "창 미상은 200K 로 본다(보수)");
        assert_eq!(ctx_floor_ceiling(Some(0)), ctx_floor_ceiling(W200K), "창 0 은 미상이다(결측은 값이 아니다)");
        assert!(ctx_floor_ceiling(W200K) < 83, "200K 천장이 선제 압축점 83.5% 이상");
        for w in [40_000u64, 48_000, 100_000, 128_000, 200_000, 272_000, 400_000, 500_000, 1_000_000, 2_000_000] {
            let c = ctx_floor_ceiling(Some(w));
            assert!(c <= CTX_FLOOR_CEIL, "창 {w}: 천장 {c} 가 85 캡을 넘는다");
            if c == 0 {
                continue;
            }
            let fire = max_tokens_shown_as(c, w);
            let compact_at = w.saturating_sub(CC_SUMMARY_RESERVE_TOKENS + CC_AUTOCOMPACT_BUFFER_TOKENS);
            let block_at = w.saturating_sub(CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS);
            assert!(fire + CTX_FLOOR_CYCLE_MARGIN_TOKENS <= compact_at, "창 {w}: 천장 {c}% + 사이클 여유가 선제 압축점을 넘는다");
            assert!(fire + CTX_FLOOR_CYCLE_MARGIN_TOKENS < block_at, "창 {w}: 천장 {c}% + 사이클 여유가 차단점에 닿는다");
        }
        assert_eq!(ctx_floor_ceiling(Some(40_000)), 0, "창이 예약보다 작으면 올릴 곳이 없다(발화)");
    }

    /// ★(RR3-R1-1) 차단 상한 — 그 퍼센트에서 발화해도 CSO 사이클(여유 15000 토큰)이 Claude Code 차단점(창−23000) 전에
    /// 끝난다. 200K 80 · 1M 95 · 미상 = 200K · 언제나 선제 압축 천장 이상 · 선제 압축점(83.5%) 아래(교차가 압축보다 먼저).
    #[test]
    fn ctx_floor_hard_cap_keeps_the_cycle_before_the_blocking_point() {
        assert_eq!(ctx_floor_hard_cap(W200K), 80, "200K 차단 상한");
        assert_eq!(ctx_floor_hard_cap(W1M), 95, "1M 차단 상한");
        assert_eq!(ctx_floor_hard_cap(None), ctx_floor_hard_cap(W200K), "창 미상은 200K 로 본다(보수)");
        assert_eq!(ctx_floor_hard_cap(Some(0)), ctx_floor_hard_cap(W200K));
        assert!(max_tokens_shown_as(ctx_floor_hard_cap(W200K), 200_000) < 167_000,
                "200K 차단 상한이 선제 압축점 위라 교차가 압축에 가린다");
        for w in [40_000u64, 48_000, 100_000, 128_000, 200_000, 272_000, 400_000, 500_000, 1_000_000, 2_000_000] {
            let (c, h) = (ctx_floor_ceiling(Some(w)), ctx_floor_hard_cap(Some(w)));
            assert!(h >= c, "창 {w}: 차단 상한 {h} < 천장 {c}");
            assert!(h <= 100);
            if h == 0 {
                continue;
            }
            let block_at = w.saturating_sub(CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS);
            if h > c {
                assert!(max_tokens_shown_as(h, w) + CTX_FLOOR_CYCLE_MARGIN_TOKENS < block_at,
                        "창 {w}: 차단 상한 {h}% + 사이클 여유가 차단점에 닿는다");
            }
        }
    }

    // ───────────── 히스테리시스 막대(순수) ─────────────

    /// 대표값 — 200K: 바닥 67·70 → 75(Raise) · 71·75 → 80(Limited) · 76·78 → 80 1회(Probe) → 확인 뒤 바닥+10(Stopped) ·
    /// 91 확인 → 발화 없음. 1M: 바닥 13 → 기본 60 · 78 → 85 · 81 → 95(Limited). 창 미상은 200K 와 같다.
    #[test]
    fn ctx_floor_bar_maps_measured_floors_to_regimes() {
        use CtxFloorRegime::*;
        for win in [W200K, None] {
            assert_eq!(ctx_floor_bar(57, 60, win, false), (72, Raise));
            assert_eq!(ctx_floor_bar(67, 60, win, false), (75, Raise));
            assert_eq!(ctx_floor_bar(70, 60, win, false), (75, Raise));
            assert_eq!(ctx_floor_bar(71, 60, win, false), (80, Limited));
            assert_eq!(ctx_floor_bar(75, 60, win, true), (80, Limited), "Limited 는 확인과 무관");
            assert_eq!(ctx_floor_bar(76, 60, win, false), (80, Probe));
            assert_eq!(ctx_floor_bar(78, 60, win, false), (80, Probe));
            assert_eq!(ctx_floor_bar(80, 60, win, false), (81, Probe));
            assert_eq!(ctx_floor_bar(76, 60, win, true), (86, Stopped));
            assert_eq!(ctx_floor_bar(78, 60, win, true), (88, Stopped));
            assert_eq!(ctx_floor_bar(91, 60, win, true), (CTX_FLOOR_NEVER, Stopped));
            assert_eq!(ctx_floor_bar(100, 60, win, false), (CTX_FLOOR_NEVER, Probe));
            assert_eq!(ctx_floor_bar(30, 60, win, false), (60, Raise), "바닥이 낮으면 기본 임계(무변화)");
        }
        assert_eq!(ctx_floor_bar(13, 60, W1M, false), (60, Raise), "1M 좌석의 60% 정책 무변화");
        assert_eq!(ctx_floor_bar(78, 60, W1M, false), (85, Raise));
        assert_eq!(ctx_floor_bar(81, 60, W1M, false), (95, Limited));
        assert_eq!(ctx_floor_bar(67, 90, W200K, false), (90, Raise), "역할 override 가 더 높으면 그것");
    }

    /// ★성질 핀 — 바닥 0~100 × 기본 임계 × 창 × 확인 전수:
    /// (I1) 실효 임계가 기본보다 높으면 잰 바닥 위로 최소 여유(Probe 만 1%p) — 유휴 좌석은 바닥에서 발화할 수 없다.
    /// (I2) 실효 임계 ≤ max(기본, 바닥 + ROOM) — 바닥 위로 ROOM 만큼 실제로 자란 컨텍스트는 발화한다(100 을 넘는 것만 예외).
    /// 영역별 상한: Raise ≤ 천장 · Limited ≤ 차단 상한 · Probe ≤ max(차단 상한, 바닥+1).
    #[test]
    fn ctx_floor_bar_never_fires_at_the_floor_and_always_fires_on_real_growth() {
        use CtxFloorRegime::*;
        for win in WINDOWS {
            let (c, h) = (ctx_floor_ceiling(win), ctx_floor_hard_cap(win));
            for base in [1u8, 40, 60, 75, 80, 90, 100] {
                for floor in 0u8..=100 {
                    for confirmed in [false, true] {
                        let (bar, regime) = ctx_floor_bar(floor, base, win, confirmed);
                        let ctx = format!("창 {win:?} 기본 {base} 바닥 {floor} 확인 {confirmed}: {bar} {regime:?}");
                        assert!(bar >= base, "{ctx}");
                        if bar > base && bar < CTX_FLOOR_NEVER {
                            let min_room = if regime == Probe { 1 } else { CTX_FLOOR_MIN_ROOM };
                            assert!(u16::from(bar) >= u16::from(floor) + u16::from(min_room), "(I1) {ctx}");
                        }
                        let room_bar = (u16::from(floor) + u16::from(CTX_FLOOR_ROOM)).min(u16::from(CTX_FLOOR_NEVER)) as u8;
                        assert!(bar <= base.max(room_bar), "(I2) {ctx}");
                        match regime {
                            Raise => assert!(bar <= base.max(c), "{ctx}"),
                            Limited => assert!(bar <= base.max(h) && floor + CTX_FLOOR_MIN_ROOM > c, "{ctx}"),
                            Probe => assert!(!confirmed && bar <= base.max(h.max(floor.saturating_add(1))), "{ctx}"),
                            Stopped => assert!(confirmed && floor + CTX_FLOOR_MIN_ROOM > h, "{ctx}"),
                            // 순수 `ctx_floor_bar` 는 Backlog 를 내지 않는다(복원 끝이 필요 — `ctx_floor_bar_measured`).
                            Backlog => panic!("ctx_floor_bar 가 Backlog — {ctx}"),
                        }
                    }
                }
            }
        }
    }

    // ───────────── 좌석 모형 — ① 유휴 고리 0 · ② 필요 clear 누락 0 ─────────────

    /// ★(RR3-R1-1 · G3ROLE-1 · ①) **유휴 좌석은 부트 첫 교차 1회 뒤 다시 발화하지 않는다** — clear 뒤 바닥 0~100% ×
    /// 복원 성장(0·4·8.45·12%p) × 복원 시간(40·240·500초) × 창(200K·1M·미상) × 기본 임계(60·90), 유휴 3시간. 단 하나의
    /// 예외: 붙여넣기·복원만으로 max(기본, 차단 상한)을 넘는 좌석은 정착 창 뒷문이 **1회** 더 clear 해 보고 멈춘다(총 2회).
    /// 종전(52b8c656)은 200K 바닥 71 이상에서 CSO 사이클마다(여기선 약 90초마다) 끝없이 발화했다.
    #[test]
    fn ctx_loop_guard_idle_seat_never_reenters_the_clear_loop() {
        for win in WINDOWS {
            for base in [60u8, 90] {
                for paste in (0..=100).step_by(1) {
                    for (restore, restore_secs) in [(0.0, 40.0), (4.0, 40.0), (8.45, 240.0), (12.0, 500.0)] {
                        let seat = Seat { restore_secs, ..Seat::idle(base, win, f64::from(paste), restore) };
                        let run = run_seat(seat, 3.0 * 3600.0);
                        let floor = (f64::from(paste) + restore).min(100.0).round().max(3.0) as u8;
                        let expect = if floor >= base.max(ctx_floor_hard_cap(win)) { 2 } else { 1 };
                        assert_eq!(run.fires.len(), expect,
                                   "창 {win:?} 기본 {base} 붙여넣기 {paste} 복원 +{restore}/{restore_secs}s: 유휴 좌석 발화 {:?}",
                                   run.fires);
                    }
                }
            }
        }
    }

    /// ★(② 무clear 금지) **바닥 위로 실제로 자란 컨텍스트는 발화한다** — 같은 전수에서 복원·정착 뒤 쉬었다가 30초마다
    /// 약 0.5%p 씩 자라는 좌석: 첫 clear 뒤 발화가 반드시 나고, 그 pct 는 max(기본, 바닥 + ROOM) 이하(100 초과 제외)·
    /// 영역의 상한 이하이며, 바닥 위 성장은 최소 여유 이상(Probe 는 1%p 이상)이다.
    #[test]
    fn ctx_loop_guard_clears_every_seat_that_really_grows_past_its_floor() {
        for win in WINDOWS {
            for base in [60u8, 90] {
                for paste in 0..=100u8 {
                    let seat = Seat { work_after: 700.0, work_per_min: 1.0, ..Seat::idle(base, win, f64::from(paste), 0.0) };
                    let run = run_seat(seat, 4.0 * 3600.0);
                    let floor = paste.max(3); // 붙여넣기 전 clear 직후 3% 도 정착 창 안이다
                    let (bar, regime) = ctx_floor_bar(floor, base, win, false);
                    let ctx = format!("창 {win:?} 기본 {base} 바닥 {floor} → {bar} {regime:?}: {:?}", run.fires);
                    if floor >= base.max(ctx_floor_hard_cap(win)) {
                        // 붙여넣기만으로 뒷문 높이 위 — 정착 창 뒷문 1회(Probe) → 다음 세션은 증명·확인된 Stopped(바닥+10).
                        assert_eq!(run.fires.get(1).map(|f| (f.1, f.2)), Some((floor, backstop(floor, CtxFloorRegime::Probe).unwrap())), "{ctx}");
                        let stop = u16::from(floor) + u16::from(CTX_FLOOR_MIN_GROWTH);
                        if stop <= 100 {
                            assert_eq!(run.fires.get(2).map(|f| (f.1, f.2)), Some((stop as u8, post(floor, CtxFloorRegime::Stopped).unwrap())),
                                       "(②) Stopped 좌석이 바닥+10 까지 자랐는데 clear 되지 않았다 — {ctx}");
                        } else {
                            assert_eq!(run.fires.len(), 2, "{ctx}");
                        }
                        continue;
                    }
                    if bar > 100 {
                        assert_eq!(run.fires.len(), 1, "바닥 {floor}: 닿을 수 없는 임계에서 발화했다");
                        continue;
                    }
                    assert!(run.fires.len() >= 2, "(②) 자란 좌석이 clear 되지 않았다 — {ctx}");
                    let (_, p, f, _) = run.fires[1];
                    assert_eq!(f.floor, Some(floor), "{ctx}");
                    assert!(p >= bar && p <= bar + 1, "(②) 실효 임계 {bar} 에서 발화하지 않았다(pct {p}) — {ctx}");
                    let room_bar = (u16::from(floor) + u16::from(CTX_FLOOR_ROOM)) as u8;
                    assert!(p <= base.max(room_bar) + 1, "(②) 바닥+ROOM 을 넘도록 끌었다 — {ctx}");
                    if bar > base {
                        let min_room = if regime == CtxFloorRegime::Probe { 1 } else { CTX_FLOOR_MIN_ROOM };
                        assert!(p >= floor + min_room, "(①) 바닥 위 성장 {}%p 로 발화 — {ctx}", p - floor);
                    }
                }
            }
        }
    }

    /// ★(① 작업이 사이클을 부른다) 일하는 좌석의 사이클 수는 실제 성장량 ÷ 최소 여유로 묶인다 — 200K master(바닥 63·72)·
    /// CEO(바닥 69→복원 78)·worker(바닥 30) · 1M 각각 분당 0.2~2%p 로 6시간. 모든 clear 뒤 발화는 바닥 위 최소 여유 이상
    /// 자란 뒤이고(Probe 는 연속 구간당 1회만 1%p 이상), 좌석이 실효 임계를 한 보고 넘게 넘어 끌려가지 않는다(②).
    #[test]
    fn ctx_loop_guard_working_seats_cycle_only_as_often_as_they_grow() {
        let profiles = [
            ("200K master", W200K, 63.3, 8.9),
            ("200K master 72", W200K, 72.0, 0.0),
            ("200K CEO", W200K, 68.9, 8.45),
            ("200K worker", W200K, 25.0, 5.0),
            ("1M master", W1M, 13.0, 2.0),
            ("미상 CEO", None, 68.9, 8.45),
        ];
        for (name, win, paste, restore) in profiles {
            for rate in [0.2, 0.5, 1.0, 2.0] {
                let seat = Seat { restore_secs: 240.0, work_after: 0.0, work_per_min: rate, ..Seat::idle(60, win, paste, restore) };
                let run = run_seat(seat, 6.0 * 3600.0);
                let mut backstops_in_a_row = 0;
                let mut prev_tier3 = false;
                for (t, p, f, _) in run.fires.iter().skip(1) {
                    let tier3 = f.settle_backstop || matches!(f.regime, Some(CtxFloorRegime::Probe | CtxFloorRegime::Stopped));
                    assert!(!(prev_tier3 && f.regime == Some(CtxFloorRegime::Probe)),
                            "{name} 분당 {rate}: 차단 상한 근처 세션 뒤 또 Probe — 바닥+1%p 마다 도는 고리(①) {:?}", run.fires);
                    prev_tier3 = tier3;
                    let floor = f.floor.expect("clear 뒤 발화에는 잰 바닥이 있다");
                    let ctx = format!("{name} 분당 {rate}: t={t} pct={p} {f:?}");
                    if f.settle_backstop {
                        // 정착 창 뒷문 — clear 직후 차단 상한을 넘게 빠르게 자랐다(연속 세션에서 두 번 쓰지 않는다).
                        backstops_in_a_row += 1;
                        assert!(backstops_in_a_row <= 1 && *p >= ctx_floor_hard_cap(win), "{ctx}");
                        continue;
                    }
                    backstops_in_a_row = 0;
                    let min_room = if f.regime == Some(CtxFloorRegime::Probe) { 1 } else { CTX_FLOOR_MIN_ROOM };
                    assert!(p.saturating_sub(floor) >= min_room, "(①) 바닥 {floor} 위 {}%p 로 발화 — {ctx}", p.saturating_sub(floor));
                }
                // 사이클 수는 성장량이 부른다(1%p 당 1회를 넘지 않는다 — 시간·경보 쿨다운이 아니라).
                let grown = rate * 6.0 * 60.0;
                assert!((run.fires.len() as f64) <= 2.0 + grown,
                        "{name} 분당 {rate}: 사이클 {} 회 > 성장 {grown}%p — 작업보다 사이클이 많다", run.fires.len());
                assert!(run.max_pct <= 100);
            }
        }
    }

    /// ★(RR3-R1-1 드릴 · 회귀 핀) 200K master 가 바닥 72 로 clear 된 뒤 40분 유휴 — 사이클 0(종전 7회/40분 · HEAD~1 1회).
    /// 바닥 72 는 천장 75 아래 여유가 3%p 뿐이라 차단 상한 80 에서 clear 한다(Limited — 작업 8%p 마다).
    #[test]
    fn ctx_loop_guard_rr3_drill_idle_200k_master_at_floor_72_gets_no_cycle() {
        for win in [W200K, None] {
            let run = run_seat(Seat::idle(60, win, 72.0, 0.0), 40.0 * 60.0);
            assert_eq!(run.fires.len(), 1, "창 {win:?}: {:?}", run.fires);
        }
        // 같은 좌석이 일하면 80% 에서 clear(바닥 72 위 8%p) — 무clear 아님.
        let seat = Seat { work_after: 400.0, work_per_min: 1.0, ..Seat::idle(60, W200K, 72.0, 0.0) };
        let run = run_seat(seat, 40.0 * 60.0);
        assert!(run.fires.len() >= 2, "{:?}", run.fires);
        assert_eq!((run.fires[1].1, run.fires[1].2), (80, post(72, CtxFloorRegime::Limited).unwrap()));
    }

    /// ★(G3ROLE-1) 200K CEO — 재주입 68.9% → 5분 안 복원 +8.45%p(77.35%): 종전은 교차 시점(69) 기준으로 75 로 올린 뒤
    /// 복원 도중 75% 에서 작업 0 으로 발화했다(사이클마다). 이제 복원 끝까지 바닥으로 재므로(78 — 차단 상한 근처) 유휴면
    /// 발화 0 · 일하면 1회 재시도(80%) 뒤 자동 clear 를 멈추고(Stopped) 바닥+10(88%)에서만 다시 clear 한다.
    /// master(63.3 → 72.2)는 Limited — 80% 까지 약 8%p 일한 뒤에만 clear.
    #[test]
    fn ctx_loop_guard_g3role_restore_growth_is_part_of_the_floor() {
        let ceo = Seat { restore_secs: 30.0, ..Seat::idle(60, W200K, 68.9, 8.45) };
        assert_eq!(run_seat(ceo, 3600.0).fires.len(), 1, "200K CEO 유휴 재발화");
        let run = run_seat(Seat { work_after: 700.0, work_per_min: 0.5, ..ceo }, 4.0 * 3600.0);
        let post_clear: Vec<_> = run.fires.iter().skip(1).map(|(_, p, f, _)| (*p, f.regime)).collect();
        assert_eq!(post_clear.first(), Some(&(80, Some(CtxFloorRegime::Probe))), "{post_clear:?}");
        assert!(post_clear.iter().skip(1).all(|(p, r)| *r == Some(CtxFloorRegime::Stopped) && *p >= 87),
                "Probe 뒤에는 Stopped(바닥+10)에서만 clear — {post_clear:?}");
        let master = Seat { restore_secs: 240.0, ..Seat::idle(60, W200K, 63.3, 8.9) };
        assert_eq!(run_seat(master, 3600.0).fires.len(), 1, "200K master 유휴 재발화");
        let run = run_seat(Seat { work_after: 60.0, work_per_min: 0.5, ..master }, 3.0 * 3600.0);
        assert!(run.fires.len() >= 3, "{:?}", run.fires);
        for (_, p, f, restored) in run.fires.iter().skip(1) {
            assert_eq!((*p, f.regime), (80, Some(CtxFloorRegime::Limited)), "{:?}", run.fires);
            assert!(f64::from(*p) - restored >= 7.0, "master 가 복원 뒤 7%p 도 일하지 않고 clear 됐다: {:?}", run.fires);
            assert!(p - f.floor.unwrap() >= CTX_FLOOR_MIN_ROOM, "{:?}", run.fires);
        }
    }

    /// ★(자기 반례 · ①) 차단 상한 근처 바닥(200K 78)의 **바쁜** 좌석 — 분당 0.3~1%p 가 복원 직후부터 이어진다(주기 신호·
    /// 관리 작업). 재시도(Probe · 80%)는 1회뿐이고 그 뒤 세션은 확인된 Stopped(바닥+10)라, 작업 1~2%p 마다 지침 전문을
    /// 다시 붙여 넣는 사이클이 없다(흡수한 작업이 바닥을 올려도 마찬가지).
    #[test]
    fn ctx_loop_guard_busy_seat_near_the_hard_cap_does_not_probe_every_cycle() {
        for rate in [0.3, 0.5, 1.0] {
            let seat = Seat { restore_secs: 30.0, work_after: 0.0, work_per_min: rate, ..Seat::idle(60, W200K, 70.0, 8.0) };
            let run = run_seat(seat, 6.0 * 3600.0);
            let probes = run.fires.iter().filter(|f| f.2.regime == Some(CtxFloorRegime::Probe) && !f.2.settle_backstop).count();
            let backstops = run.fires.iter().filter(|f| f.2.settle_backstop).count();
            assert!(probes + backstops <= 1, "분당 {rate}: 재시도가 {probes}+{backstops} 회 — {:?}", run.fires);
            for (_, p, f, _) in run.fires.iter().skip(2) {
                assert_eq!(f.regime, Some(CtxFloorRegime::Stopped), "분당 {rate}: {:?}", run.fires);
                assert!(p - f.floor.unwrap() >= CTX_FLOOR_MIN_GROWTH, "분당 {rate}: {:?}", run.fires);
            }
        }
    }

    /// ★(자기 반례 · ② · 차단기 오판) 바닥은 **clear 사이클 자신의 입력(붙여넣기 → 복원 턴)이 끝난 뒤** 정해진다 — 복원
    /// 턴이 끝나 좌석 출력이 조용해지면 정착 창을 닫는다. 종전(시간 창만 · 오르는 동안 최대 600초)은 복원 뒤 곧바로 들어온
    /// 작업(대기열 배달·push 한 턴씩)을 바닥에 흡수했다 — 바쁜 200K master(참 바닥 72)가 77~85 로 재여 Probe → Stopped
    /// (바닥+10 = 86~95%)로 오판됐다(샌드박스 final-m200-work 94.9% · final-c200-work 97.9%): 선제 압축점(83.5%) 위라 cys
    /// clear 가 안 나거나(Claude 압축이 먼저) 자동 압축을 끈 좌석은 차단점(88.5%)에서 멈춘다(R2NC3-1 의 재발).
    /// 참 바닥에 여유가 있는 좌석은 일하는 동안 **언제나** 정상·Limited 영역에서 max(기본, 차단 상한) 이하로 clear 된다.
    /// 복원이 최소 창([`CTX_FLOOR_SETTLE_MIN_SECS`])보다 짧고 작업이 그 안에 오면 그 몫(분당 성장 × 최소 창 + 한 턴)만 든다.
    #[test]
    fn ctx_loop_guard_busy_seat_floor_is_the_restore_turn_not_the_work_after_it() {
        let profiles = [
            ("200K master", W200K, 63.3, 8.9, 80u8),
            ("미상 master", None, 63.3, 8.9, 80),
            ("200K master 70", W200K, 66.0, 4.0, 80),
            ("200K worker", W200K, 25.0, 5.0, 60),
            ("1M CEO", W1M, 14.0, 1.7, 60),
        ];
        for (name, win, paste, restore, cap) in profiles {
            for rate in [0.5, 1.0, 2.0] {
                for turn_secs in [20.0, 45.0] {
                    for restore_secs in [30.0, 120.0, 280.0] {
                        let seat = Seat { restore_secs, work_after: 5.0, work_per_min: rate, turn_secs,
                                          ..Seat::idle(60, win, paste, restore) };
                        let run = run_seat(seat, 6.0 * 3600.0);
                        let restored = (paste + restore).round() as u8;
                        // 복원이 최소 창보다 짧으면 그 안에 든 작업(최소 창 + 한 턴)까지는 바닥이다.
                        let absorbed = if restore_secs < CTX_FLOOR_SETTLE_MIN_SECS {
                            (rate * (CTX_FLOOR_SETTLE_MIN_SECS / 60.0 + 1.0)).ceil() as u8
                        } else {
                            0
                        };
                        let ctx = format!("{name} 분당 {rate} 턴 {turn_secs}s 복원 {restore_secs}s: {:?}", run.fires);
                        assert!(run.fires.len() >= 3, "(②) 일하는 좌석이 clear 되지 않았다 — {ctx}");
                        for (_, p, f, _) in run.fires.iter().skip(1) {
                            let floor = f.floor.expect("clear 뒤 발화에는 잰 바닥이 있다");
                            assert!(floor <= restored + absorbed + 1,
                                    "바닥에 복원 턴 뒤 작업이 흡수됐다(잰 {floor} · 복원 끝 {restored}) — {ctx}");
                            assert!(matches!(f.regime, Some(CtxFloorRegime::Raise | CtxFloorRegime::Limited)),
                                    "차단기 오판 — 여유 있는 바쁜 좌석이 Probe·Stopped 로 갔다 — {ctx}");
                            assert!(!f.settle_backstop, "{ctx}");
                            assert!(*p <= cap + 1, "(②) {p}% 까지 clear 가 늦었다(상한 {cap}) — {ctx}");
                        }
                        assert!(run.max_post_clear <= cap + 1, "(②) 좌석이 {}% 까지 끌려갔다 — {ctx}", run.max_post_clear);
                    }
                }
            }
        }
    }

    /// ★(자기 반례 탐색 · 결정론 난수 1500 좌석) 붙여넣기 0~95% × 복원 0~15%p·20~300초 × 창(200K·1M·미상) × 기본 임계
    /// 60·75 × 작업 0~3%p/분 · 턴 10~50초(매 분 · 턴 사이는 조용) × 사이클 지연 20~120초, 3시간:
    /// (①) 유휴면 부트 1회(+ 붙여넣기·복원만으로 뒷문 높이 위면 1회) · 일하면 모든 clear 뒤 발화가 잰 바닥 위로 최소 여유
    ///     (Probe 1%p) 이상 자란 뒤(뒷문 제외)이고 사이클 수 ≤ 2 + 실제 성장(%p).
    /// (② · 오판) 참 바닥(복원 끝) + 흡수 몫(복원이 최소 창보다 짧을 때만) + 1 에 최소 여유가 있는 좌석은 Raise·Limited 로만
    ///     clear 되고 그 pct 는 max(기본, 차단 상한) + 1 이하 · 잰 바닥은 참 바닥 + 흡수 몫 + 1 이하.
    #[test]
    fn ctx_loop_guard_randomized_seats_keep_both_invariants() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = |lo: f64, hi: f64| {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            lo + (hi - lo) * ((state >> 11) as f64 / (1u64 << 53) as f64)
        };
        for case in 0..1500 {
            let win = WINDOWS[(rnd(0.0, 3.0) as usize).min(2)];
            let base = if rnd(0.0, 1.0) < 0.8 { 60 } else { 75 };
            let paste = rnd(0.0, 95.0).round();
            let restore = if rnd(0.0, 1.0) < 0.2 { 0.0 } else { rnd(0.0, 15.0) };
            let restore_secs = rnd(20.0, 300.0);
            let work_per_min = if rnd(0.0, 1.0) < 0.3 { 0.0 } else { rnd(0.1, 3.0) };
            let turn_secs = rnd(10.0, 50.0);
            let seat = Seat { base, window: win, paste, restore, restore_secs, work_after: rnd(5.0, 60.0), work_per_min, turn_secs,
                              cycle_delay: rnd(20.0, 120.0), quiesce: rnd(5.0, 30.0), ..Seat::idle(base, win, paste, restore) };
            let run = run_seat(seat, 3.0 * 3600.0);
            let ctx = format!("case {case} {seat:?}: {:?}", run.fires);
            let h = ctx_floor_hard_cap(win);
            let restored = (paste + restore).min(100.0).round().max(3.0) as u16;
            if work_per_min == 0.0 {
                let expect = if restored >= u16::from(base.max(h)) { 2 } else { 1 };
                assert!(run.fires.len() <= expect, "(①) 유휴 좌석 재발화 — {ctx}");
                continue;
            }
            let grown = work_per_min * 3.0 * 60.0;
            assert!((run.fires.len() as f64) <= 2.0 + grown, "(①) 사이클이 성장보다 많다 — {ctx}");
            let absorbed = if restore_secs < CTX_FLOOR_SETTLE_MIN_SECS {
                (work_per_min * (CTX_FLOOR_SETTLE_MIN_SECS / 60.0 + 1.0)).ceil() as u16
            } else {
                0
            };
            let roomy = restored + absorbed + 1 + u16::from(CTX_FLOOR_MIN_ROOM) <= u16::from(h);
            for (_, p, f, _) in run.fires.iter().skip(1) {
                let floor = u16::from(f.floor.expect("clear 뒤 발화의 잰 바닥"));
                let min_room = if f.regime == Some(CtxFloorRegime::Probe) { 1 } else { u16::from(CTX_FLOOR_MIN_ROOM) };
                if !f.settle_backstop && f.regime != Some(CtxFloorRegime::Stopped) && u16::from(*p) < 100 {
                    assert!(u16::from(*p) >= floor + min_room.min(100 - floor), "(①) 바닥 {floor} 위 {p} — {ctx}");
                }
                assert!(floor <= restored + absorbed + 1, "잰 바닥 {floor} > 참 바닥 {restored} + 흡수 {absorbed} + 1 — {ctx}");
                if roomy {
                    assert!(matches!(f.regime, Some(CtxFloorRegime::Raise | CtxFloorRegime::Limited)), "(오판) {ctx}");
                    assert!(u16::from(*p) <= u16::from(base.max(h)) + 1, "(②) {p}% 까지 clear 가 늦었다 — {ctx}");
                }
            }
        }
    }

    /// 정착 창의 조용함 닫힘(`note_idle`) 세부 — 닫는 조건 넷(틈 길이 ≥ 2초 · 시작점 **뒤**에 시작 · 시작점 + 최소 창 이후까지
    /// 이어짐 · 시작점 뒤 관측 있음)이 다 있어야 닫고, 닫힌 뒤의 관측은 바닥에 들지 않으며(막대는 닫힐 때 바닥에서) 정착 창
    /// 뒷문도 없다. 다음 세션 교체는 다시 연다. 조용함 신호가 없으면(`NONE`) 종전 시간 창 그대로(실패 방향 — 늦게 닫힘).
    #[test]
    fn ctx_loop_guard_idle_closes_the_settle_window_only_after_the_restore_turn() {
        let setup = || {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
            clear(&mut g, &mut armed, 100.0);
            g.note_quiescing(110.0); // 사이클이 붙여넣기까지 — 시작점 110
            (g, armed)
        };
        let min = CTX_FLOOR_SETTLE_MIN_SECS;
        // ① 시작점 뒤 관측(복원 끝 72) + 최소 창 뒤까지 이어진 조용함 → 닫힘.
        let (mut g, mut armed) = setup();
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 150.0), None);
        g.note_idle(150.0, 110.0 + min - 1.0, 110.0 + min - 1.0);
        assert!(g.settling(110.0 + min - 1.0), "최소 창 전의 조용함이 창을 닫았다");
        g.note_idle(150.0, 110.0 + min, 110.0 + min);
        assert!(!g.settling(110.0 + min), "복원 턴이 끝난 조용함이 창을 닫지 않았다");
        // 닫힌 뒤 작업(75·79)은 바닥에 들지 않는다 — 막대는 닫힌 바닥 72 에서(Limited 80) · 뒷문 없음.
        assert_eq!(report(&mut g, &mut armed, 75, 60, W200K, 200.0), None);
        assert_eq!(report(&mut g, &mut armed, 79, 60, W200K, 230.0), None);
        assert_eq!(g.settle_peak, Some(72), "닫힌 뒤 작업이 바닥에 들었다");
        assert_eq!(g.effective_threshold(60, W200K, 230.0), 80);
        assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, 260.0), post(72, CtxFloorRegime::Limited), "(②) 닫힌 바닥 위 성장");
        // ② 짧은 틈(< 2초)·시작점 전에 시작한 틈·시작점 뒤 관측 없음은 닫지 않는다.
        let (mut g, mut armed) = setup();
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 150.0), None);
        g.note_idle(300.0, 300.0 + CTX_FLOOR_IDLE_QUIET_SECS - 0.1, 301.0);
        g.note_idle(105.0, 400.0, 400.0);
        assert!(g.settling(400.0), "짧은 틈·시작점 전(사이클 중)부터의 조용함이 창을 닫았다");
        let (mut g, mut armed) = setup();
        assert_eq!(report(&mut g, &mut armed, 3, 60, W200K, 105.0), None); // 시작점 전 관측(clear 직후 3%)만
        g.note_idle(115.0, 300.0, 300.0);
        assert!(g.settling(300.0), "붙여넣기 턴의 보고 전에 창이 닫혔다(바닥 3% → 작업 0 발화 ①)");
        // ③ 다음 세션 교체는 다시 연다 — 닫힘은 세션마다.
        let (mut g, mut armed) = setup();
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 150.0), None);
        g.note_idle(150.0, 300.0, 300.0);
        assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, 400.0), post(72, CtxFloorRegime::Limited));
        clear(&mut g, &mut armed, 500.0);
        assert!(g.settling(505.0) && !g.settle_closed, "새 세션의 정착 창이 닫힌 채다");
        // ④ 조용함 신호 없음 — 종전 시간 창(기본 300초 · 오르는 동안 최대 600초) 그대로.
        let (mut g, mut armed) = setup();
        let n = CtxIdleObs::NONE;
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 150.0), None);
        g.note_idle(n.quiet_since, 300.0, 300.0);
        assert!(g.settling(300.0) && !g.settling(110.0 + CTX_FLOOR_SETTLE_SECS + 1.0), "신호 없음이 시간 창을 바꿨다");
    }

    // ───────────── ★(ROLE-R4-1 · R2NC5-1) 복원 끝 바닥 · 사이클이 붙잡은 배달 · Claude 압축 재무장 ─────────────

    fn tier3_floor(floor: u8, window: Option<u64>) -> bool {
        ctx_floor_bar(floor, 0, window, false).1.tier3()
    }

    /// ★(ROLE-R4-1 · R2NC5-1 · RV2NC-E20-1 · 성질 핀) 복원 끝 바닥은 **차단기만 풀고 어떤 막대도 앞당기지 않는다** — 정착 창
    /// 최고치 0~100 × 복원 끝(없음 · 0~최고치) × 기본 임계 × 창 × 확인 전수:
    /// (①) 막대 ≥ 정착 창 최고치의 미확인 막대(유휴 재발화 0 은 최고치 위에서 그대로 — 복원 끝 판정이 틀려 낮아도 고리 없음).
    /// (②) 막대 ≤ 종전 막대(확인 반영) + 1 — 얇은 세션(복원 끝 여유 · 최고치 차단기 높이)의 미확인 막대만 max(차단 상한, 최고치 + 2)
    ///     로 1%p 늦다(1%p 반올림에서 발화가 실제 성장 ≥ 1%p 를 입증하는 최소 막대 — 그 발화의 성장 속도로 확인을 가른다).
    /// 복원 끝 바닥에 여유가 있으면(정상·Limited 높이) Probe·Stopped 가 아니다 — 얇은 세션은 미확인이면 Limited, ★(RV2NC-E20-1)
    /// 확인(직전 clear 세션도 차단 상한 근처)이면 Backlog(최고치 + 최소 여유 — 몰림을 바닥으로 확정 · Stopped 의 + 10 보다 이르다) ·
    /// Stopped 는 복원 끝도 차단기 높이일 때만 · 복원 끝이 없거나 최고치와 같으면 종전 판정 그대로.
    #[test]
    fn ctx_floor_bar_measured_only_lifts_the_breaker_never_a_bar() {
        use CtxFloorRegime::*;
        for win in WINDOWS {
            let h = ctx_floor_hard_cap(win);
            for base in [1u8, 40, 60, 75, 80, 90, 100] {
                for settled in 0u8..=100 {
                    for confirmed in [false, true] {
                        let (lo, _) = ctx_floor_bar(settled, base, win, false);
                        let old = ctx_floor_bar(settled, base, win, confirmed);
                        assert_eq!(ctx_floor_bar_measured(settled, None, base, win, confirmed),
                                   CtxFloorBar { bar: old.0, regime: old.1, floor: settled, settled, thin: false, tainted: false, lift: None },
                                   "복원 끝 없음 = 종전");
                        for r in 0u8..=settled {
                            let m = ctx_floor_bar_measured(settled, Some(r), base, win, confirmed);
                            let ctx = format!("창 {win:?} 기본 {base} 최고치 {settled} 복원 끝 {r} 확인 {confirmed}: {m:?}");
                            assert!(m.bar >= lo && u16::from(m.bar) <= u16::from(old.0) + 1,
                                    "(①·②) 막대가 미확인 막대 {lo} ~ 종전 {} + 1 밖 — {ctx}", old.0);
                            assert_eq!((m.settled, m.floor), (settled, r), "보고 바닥은 복원 끝 — {ctx}");
                            if m.bar > base && m.bar < CTX_FLOOR_NEVER {
                                assert!(m.bar > settled, "(①) 정착 창 최고치에서 발화한다 — {ctx}");
                            }
                            if tier3_floor(r, win) {
                                assert_eq!((m.bar, m.regime, m.thin), (old.0, old.1, false), "참으로 가득 찬 좌석은 종전 그대로 — {ctx}");
                            } else {
                                assert!(!m.regime.tier3(), "여유 있는 복원 끝인데 차단기 영역 — {ctx}");
                                if old.1.tier3() {
                                    let s = u16::from(settled);
                                    let want = if confirmed {
                                        (((s + u16::from(CTX_FLOOR_MIN_ROOM) + 1).min(u16::from(CTX_FLOOR_NEVER)) as u8).max(base), Backlog)
                                    } else {
                                        ((u16::from(h).max(s + 2).min(u16::from(CTX_FLOOR_NEVER)) as u8).max(base), Limited)
                                    };
                                    assert_eq!((m.bar, m.regime, m.thin), (want.0, want.1, true), "얇은 세션 — {ctx}");
                                    assert!(m.bar <= base || u16::from(m.bar) >= u16::from(settled) + 2 || m.bar == CTX_FLOOR_NEVER,
                                            "(①) 얇은 세션의 발화가 실제 성장 1%p 를 입증하지 못한다 — {ctx}");
                                } else {
                                    assert_eq!((m.bar, m.regime, m.thin), (old.0, old.1, false), "정상·Limited 는 종전 그대로 — {ctx}");
                                }
                            }
                            if m.regime == Stopped {
                                assert!(confirmed && tier3_floor(r, win), "{ctx}");
                            }
                        }
                    }
                }
            }
        }
        // 대표값(200K) — 참 바닥 72 + 대기열 5%p(최고치 77): 종전 Probe 80 → 확인되면 Stopped 87 · 이제 미확인 Limited 80(바닥 72) ·
        // 확인(직전 세션도 느린 얇은 발화)이면 Backlog 83(몰림 77 위 실제 성장 5%p — 표시 + 6).
        assert_eq!(ctx_floor_bar(77, 60, W200K, true), (87, Stopped));
        let bar = |bar, regime, floor, settled, thin| CtxFloorBar { bar, regime, floor, settled, thin, tainted: false, lift: None };
        assert_eq!(ctx_floor_bar_measured(77, Some(72), 60, W200K, false), bar(80, Limited, 72, 77, true));
        assert_eq!(ctx_floor_bar_measured(77, Some(72), 60, W200K, true), bar(83, Backlog, 72, 77, true));
        assert_eq!(ctx_floor_bar_measured(77, Some(77), 60, W200K, true), bar(87, Stopped, 77, 77, false));
        assert_eq!(ctx_floor_bar_measured(79, Some(72), 60, W200K, false), bar(81, Limited, 72, 79, true),
                   "최고치 79 — 막대 80 은 실제 성장 0.01%p 로도 닿는다(최고치 + 2)");
        assert_eq!(ctx_floor_bar_measured(83, Some(72), 60, W200K, false), bar(85, Limited, 72, 83, true),
                   "정착 창에 차단 상한 넘게 든 작업 — 최고치 + 2");
        assert_eq!(ctx_floor_bar_measured(83, Some(72), 60, W200K, true), bar(89, Backlog, 72, 83, true));
    }

    /// ★(R2NC5-1 반례 · 재검증자 검체 — 운영 입력 기록을 더함) 여유 있는 200K master(참 바닥 72 = 붙여넣기 64 + 복원 턴 30초)가
    /// 사이클 동안 쌓인 대기열(백로그 4%p)을 복원 턴 3초 뒤 한 턴으로 받고 그 턴이 시작점 + 60초 전에 끝난다 — fba29db0 는 그
    /// 백로그를 바닥에 실어(잰 76) Probe 80 → Stopped 86 → Stopped 86(선제 압축점 83.5% 위 · cys clear 없음)이었다(적색 로그
    /// `r2-noclear-reverify-fba29db0-evidence/unit`). 배달은 writer Inject 라 그 시각이 좌석 입력으로 남는다(운영 신호 —
    /// `note_input`). 이제 세 세션 모두 Limited 80 에서 clear · 보고 바닥 72 · 정착 창 최고치 76.
    #[test]
    fn r2nc5_backlog_after_short_restore_keeps_a_roomy_seat_clearing() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        let mut t0 = 60.0;
        let mut fires = vec![];
        for cycle in 0..3 {
            clear(&mut g, &mut armed, t0);
            g.note_quiescing(t0 + 10.0); // 붙여넣기 = 시작점
            let a = t0 + 10.0;
            g.note_input(Some(a), a); // 재주입 붙여넣기 자체(시작점과 같은 때 — 복원 턴의 입력)
            // 복원 턴(30초 · 출력 계속) — 64 → 72
            assert_eq!(report(&mut g, &mut armed, 64, 60, W200K, a + 5.0), None);
            assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 30.0), None);
            // 조용함 3초 뒤 대기열 배달(좌석 입력) — 끝난 틈과 입력은 다음 보고 전에
            g.note_delivery(Some(a + 33.0), None, a + 33.0);
            g.note_idle(a + 30.0, a + 33.0, a + 33.0);
            // 백로그 한 턴(12초) — 72 → 76
            assert_eq!(report(&mut g, &mut armed, 74, 60, W200K, a + 40.0), None);
            assert_eq!(report(&mut g, &mut armed, 76, 60, W200K, a + 45.0), None);
            // 이어지는 조용함 — 수집기 틱(2초)
            let mut t = a + 47.0;
            while t <= a + 90.0 {
                g.note_idle(a + 45.0, t, t);
                t += 2.0;
            }
            // 이후 작업 분당 1%p — 77 → 90(가짜 Claude 압축 83.5 는 여기서 무시하고 발화 여부만 본다)
            let mut fired = None;
            for k in 0..14u8 {
                let tt = a + 120.0 + 60.0 * f64::from(k);
                g.note_delivery(Some(tt), None, tt);
                if let Some(f) = report(&mut g, &mut armed, 77 + k, 60, W200K, tt) {
                    fired = Some((77 + k, f));
                    break;
                }
            }
            fires.push((cycle, fired));
            t0 = a + 120.0 + 60.0 * 14.0 + 60.0;
        }
        for (cycle, f) in &fires {
            let (p, fire) = f.expect("발화 없음");
            assert!(p <= 81, "(②) 참 바닥 72 좌석의 clear {cycle} 뒤 세션이 {p}% 에서야 발화({fire:?}) — 선제 압축점 83.5% 위 · 전체 {fires:?}");
            assert_eq!(Some(fire), post_split(72, 76, CtxFloorRegime::Limited), "clear {cycle}: {fires:?}");
        }
    }

    /// ★(ROLE-R4-1 · R2NC5-1 · ②) 운영 신호 모형(1초 틱 · 좌석 입력 기록 · 턴 끝 보고 · Claude 선제 압축 83.5%): 여유 있는
    /// 200K·창 미상 master(참 바닥 72~74)가 **짧은 복원 턴**(라이브 14~53초) 뒤 사이클이 붙잡았던 배달 몰림(1~3건 · 3.3~7%p ·
    /// 조용함 3초마다 한 턴)을 받고 그 뒤로도 일한다 — clear 뒤 모든 발화는 Raise·Limited(차단기 0) · 차단 상한 80(+1) 이하 ·
    /// 보고 바닥은 참 바닥(+1 반올림) · 바닥 위 최소 여유 이상 자란 뒤(①)이고, Claude 압축이 cys clear 를 가로채지 않는다.
    /// fba29db0 는 같은 모양(bl2-burst3 · rv4-bl-3x7300)에서 Probe → Stopped(86~88) → Claude 압축 · cys 사이클 0 이었다.
    #[test]
    fn ctx_loop_guard_role_r4_backlog_after_a_short_restore_never_trips_the_breaker() {
        let profiles: [(&str, Option<u64>, f64, f64); 4] = [
            ("200K master", W200K, 63.3, 8.9),
            ("200K master 72", W200K, 72.0, 0.0),
            ("미상 master", None, 63.3, 8.9),
            ("200K master 74", W200K, 70.0, 4.0),
        ];
        for (name, win, paste, restore) in profiles {
            for restore_secs in [14.0, 30.0, 53.0] {
                for (backlog, turns) in [(3.3, 2u32), (5.0, 3), (5.0, 1), (7.0, 3)] {
                    let restored = (paste + restore).round();
                    if restored + backlog >= 79.0 {
                        continue; // 배달만으로 차단 상한을 넘는 몰림은 아래 별도 검체(뒷문)
                    }
                    for (rate, queue_gap) in [(0.5, 0.3), (1.0, 0.3), (0.5, QUEUE_QUIET), (1.0, QUEUE_QUIET)] {
                        let seat = Seat { restore_secs, backlog, backlog_turns: turns, work_after: 20.0, work_per_min: rate,
                                          turn_secs: 20.0, queue_gap, ..Seat::real(60, win, paste, restore) };
                        let run = run_seat(seat, 5.0 * 3600.0);
                        let ctx = format!("{name} 복원 {restore_secs}s 배달 {backlog}%p/{turns} 간격 {queue_gap}s 분당 {rate}: {:?}", run.fires);
                        assert!(run.fires.len() >= 3, "(②) 일하는 좌석이 clear 되지 않았다 — {ctx}");
                        assert!(run.compactions.is_empty(), "(②) Claude 압축이 cys clear 보다 먼저 왔다 {:?} — {ctx}", run.compactions);
                        for (_, p, f, _) in run.fires.iter().skip(1) {
                            let floor = f.floor.expect("clear 뒤 발화의 잰 바닥");
                            assert!(matches!(f.regime, Some(CtxFloorRegime::Raise | CtxFloorRegime::Limited)),
                                    "차단기 오판 — 여유 있는 좌석이 Probe·Stopped 로 갔다 — {ctx}");
                            assert!(!f.after_compaction, "{ctx}");
                            assert!(f64::from(floor) <= restored + 1.0, "보고 바닥 {floor} 에 배달이 들었다(참 바닥 {restored}) — {ctx}");
                            assert!(*p <= 81, "(②) {p}% 까지 clear 가 늦었다 — {ctx}");
                            if !f.settle_backstop {
                                assert!(p.saturating_sub(floor) >= CTX_FLOOR_MIN_ROOM, "(①) 바닥 {floor} 위 {p} — {ctx}");
                            }
                        }
                        assert!(run.max_post_clear < 83, "(②) 좌석이 선제 압축점 근처({}%)까지 끌려갔다 — {ctx}", run.max_post_clear);
                    }
                }
            }
        }
    }

    /// ★(ROLE-R4-1 · ① 고리 0) 같은 여유 있는 좌석이 배달 몰림 뒤 **유휴**면(작업 0) clear 뒤 재발화가 없다 — 배달은 잰 바닥
    /// 위 성장이지만 차단 상한(80) 아래에서 멎는다. 배달만으로 차단 상한을 넘는 몰림(참 바닥 74 + 7%p)은 정착 창 뒷문 1회
    /// (80)뒤 멈춘다(뒷문은 연속 세션에 쓰지 않는다 · 차단기 영역으로는 가지 않는다).
    #[test]
    fn ctx_loop_guard_role_r4_backlog_on_an_idle_seat_does_not_loop() {
        for win in [W200K, None] {
            for (paste, restore, backlog, turns) in [(63.3, 8.9, 5.0, 3u32), (72.0, 0.0, 5.0, 1), (70.0, 4.0, 7.0, 3), (72.0, 0.0, 9.0, 3)] {
                for (restore_secs, queue_gap) in [(14.0, 0.3), (53.0, 0.3), (14.0, QUEUE_QUIET), (53.0, QUEUE_QUIET)] {
                    let seat = Seat { restore_secs, backlog, backlog_turns: turns, queue_gap, ..Seat::real(60, win, paste, restore) };
                    let run = run_seat(seat, 3.0 * 3600.0);
                    let ctx = format!("{win:?} 참 바닥 {} + 배달 {backlog}%p/{turns} 복원 {restore_secs}s 간격 {queue_gap}s: {:?}",
                                      paste + restore, run.fires);
                    let heavy = (paste + restore).round() + backlog >= 80.0;
                    assert!(run.fires.len() <= if heavy { 2 } else { 1 }, "(①) 유휴 좌석 재발화 — {ctx}");
                    for (_, p, f, _) in run.fires.iter().skip(1) {
                        assert!(f.settle_backstop && *p >= 80 && *p <= 81, "뒷문 밖 발화 — {ctx}");
                        assert!(!f.regime.is_some_and(CtxFloorRegime::tier3), "배달 몰림이 차단기 영역으로 갔다 — {ctx}");
                    }
                    assert!(run.compactions.is_empty(), "{ctx}");
                }
            }
        }
    }

    /// ★(차단기 보존 · ①) 참으로 가득 찬 200K·창 미상 CEO(복원 끝 78 — 붙여넣기+복원만으로 차단 상한 근처)는 배달 몰림이
    /// 있어도 종전처럼 1회 재시도 뒤 자동 clear 를 멈춘다(Stopped) — 복원 끝 바닥이 차단기 높이면 판정은 종전 그대로다.
    /// Claude 선제 압축(83.5%)이 오면 그 세션은 **1회** 재무장하고(R2NC5-1 (b)) 그 사이클이 바닥을 다시 재 다시 Stopped — 사이클
    /// 수는 압축 수 + 2 를 넘지 않고 Probe 는 연속 세션에 없다. ★(ADV2-R1-1) 압축 뒤 발화는 clear 로 돌아오는 바닥(정착 창 최고치
    /// = 복원 끝 78 + 몰림) + 1 이상에서만 — cys clear 가 컨텍스트를 올리지 않는다. 그 높이가 선제 압축점 위(몰림 5%p → 84)면 압축 뒤
    /// cys 사이클이 없다(그 clear 의 붙여넣기+몰림이 곧바로 다음 압축을 부른다 · Claude 압축이 받는다).
    #[test]
    fn ctx_loop_guard_truly_full_seat_still_stops_and_rearms_only_after_claude_compaction() {
        for win in [W200K, None] {
            for (backlog, turns, queue_gap) in [(0.0, 0u32, QUEUE_QUIET), (3.3, 2, 0.3), (5.0, 3, QUEUE_QUIET), (5.0, 3, 0.3)] {
                let seat = Seat { restore_secs: 30.0, backlog, backlog_turns: turns, work_after: 20.0, work_per_min: 0.5,
                                  turn_secs: 20.0, queue_gap, ..Seat::real(60, win, 69.5, 8.45) };
                let run = run_seat(seat, 8.0 * 3600.0);
                let ctx = format!("{win:?} 배달 {backlog}%p/{turns}: 발화 {:?} 압축 {:?}", run.fires, run.compactions);
                // clear 로 돌아오는 바닥(복원 끝 + 사이클이 붙잡은 몰림).
                let landing = (69.5 + 8.45 + backlog).round() as u8;
                assert!(run.fires.len() >= if landing + 1 < 83 { 3 } else { 2 }, "(②) 일하는 좌석의 사이클이 멎었다 — {ctx}");
                assert!(run.fires.len() <= run.compactions.len() + 3, "(①) 사이클이 Claude 압축보다 많다 — {ctx}");
                assert!(run.rearms <= run.compactions.len(), "압축 없이 재무장했다 — {ctx}");
                let mut prev_probe = false;
                for (i, (t, p, f, _)) in run.fires.iter().enumerate().skip(1) {
                    let probe = f.regime == Some(CtxFloorRegime::Probe);
                    assert!(!(prev_probe && probe), "Probe 가 연속 세션에 났다(①) — {ctx}");
                    prev_probe = probe;
                    assert!(!(f.regime == Some(CtxFloorRegime::Limited) && f.settled != f.floor),
                            "가득 찬 좌석을 배달 흡수로 오판해 차단기를 풀었다 — {ctx}");
                    if f.after_compaction {
                        let prev_t = run.fires[i - 1].0;
                        assert!(run.compactions.iter().any(|c| *c > prev_t && c <= t), "압축 없는 재무장 발화 — {ctx}");
                        assert!(*p > landing, "(ADV2-R1-1) 압축 뒤 발화 {p} 가 clear 로 돌아오는 바닥 {landing} 이하 — {ctx}");
                    } else if f.regime == Some(CtxFloorRegime::Stopped) {
                        assert!(p.saturating_sub(f.floor.unwrap()) >= CTX_FLOOR_MIN_GROWTH, "{ctx}");
                    }
                }
                if landing + 1 < 83 {
                    assert!(run.fires.iter().skip(2).any(|(_, _, f, _)| f.after_compaction),
                            "Stopped 좌석이 Claude 압축 뒤에도 cys 사이클을 다시 받지 못한다(R2NC5-1 (b)) — {ctx}");
                } else {
                    assert!(!run.fires.iter().any(|(_, _, f, _)| f.after_compaction),
                            "(ADV2-R1-1) clear 바닥이 선제 압축점 위인 좌석에 압축 뒤 cys 사이클 — 그 clear 가 다음 압축을 부른다 — {ctx}");
                }
            }
        }
        // 유휴면 부트 1회뿐(압축도 없다).
        let run = run_seat(Seat { restore_secs: 30.0, backlog: 5.0, backlog_turns: 3, ..Seat::real(60, W200K, 69.5, 8.45) }, 3.0 * 3600.0);
        assert!(run.fires.len() <= 2 && run.compactions.is_empty(), "{:?}", run.fires);
    }

    /// ★(복원 끝 판정의 반례 · ①) 복원 턴 **가운데** 출력이 멎는 틈(입력 없음)은 턴 끝이 아니다 — 그 틈에서 복원 끝 바닥을
    /// 확정하면 가득 찬 좌석의 바닥이 낮게 잡혀 차단기가 풀린다. 확정은 틈 **뒤 첫 좌석 입력**에서만 한다: 입력 없이 출력이
    /// 다시 흐르면 같은 턴이다 — 복원 끝(78)까지 따라가 확정한다.
    #[test]
    fn ctx_loop_guard_restore_stall_without_input_does_not_cut_the_floor_short() {
        // 순수: 시작점 a · 복원 60 → (3초 멎음) → 70 → 78(턴 끝) → 3초 뒤 배달(입력) → 80·82.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        g.note_quiescing(110.0);
        let a = 110.0;
        g.note_input(Some(a), a);
        assert_eq!(report(&mut g, &mut armed, 60, 60, W200K, a + 5.0), None);
        g.note_idle(a + 10.0, a + 13.0, a + 13.0); // 멎음 — 입력 없음
        assert_eq!(g.restore_quiet_from, Some(a + 10.0), "후보는 선다");
        assert_eq!(g.restore_peak, None, "입력 없는 틈에서 복원 끝을 확정했다");
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, a + 20.0), None);
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, a + 30.0), None);
        g.note_delivery(Some(a + 33.0), None, a + 33.0); // 대기열 배달
        assert_eq!(g.restore_peak, Some(78), "복원 끝(78)이 아니라 멎음 시점으로 확정했다");
        assert_eq!(report(&mut g, &mut armed, 79, 60, W200K, a + 45.0), None);
        let b = g.bar(60, W200K).unwrap();
        assert_eq!((b.regime, b.floor, b.settled), (CtxFloorRegime::Probe, 78, 79), "가득 찬 좌석의 차단기가 풀렸다");
        // 모형: 가득 찬 CEO(복원 끝 78)가 복원 턴 15초째에 3초 멎어도 Probe 1회 → Stopped · 여유 있는 master(72)는 Limited 80.
        for (paste, restore, full) in [(69.5, 8.45, true), (63.3, 8.9, false)] {
            let seat = Seat { restore_secs: 40.0, stall: Some((15.0, 3.0)), backlog: 5.0, backlog_turns: 3, work_after: 20.0,
                              work_per_min: 0.5, turn_secs: 20.0, ..Seat::real(60, W200K, paste, restore) };
            let run = run_seat(seat, 5.0 * 3600.0);
            let regimes: Vec<_> = run.fires.iter().skip(1).filter(|f| !f.2.after_compaction).map(|f| f.2.regime).collect();
            if full {
                assert_eq!(regimes.first(), Some(&Some(CtxFloorRegime::Probe)), "{:?}", run.fires);
                assert!(regimes.iter().skip(1).all(|r| *r == Some(CtxFloorRegime::Stopped)), "{:?}", run.fires);
            } else {
                assert!(regimes.len() >= 2 && regimes.iter().all(|r| *r == Some(CtxFloorRegime::Limited)), "{:?}", run.fires);
                assert!(run.compactions.is_empty(), "{:?}", run.compactions);
            }
        }
    }

    /// ★(ROLE-R4-1 · 샌드박스 실측 반례) claude 좌석의 대기열은 **프롬프트 경계**(바쁨 표지 없는 빈 입력줄)에서 조용함을
    /// 기다리지 않고 배달한다 — 복원 턴 끝과 첫 배달 사이에 2초 틈이 없을 수 있다(드릴 0.1~0.5초). 복원 턴의 보고 뒤 첫 대기열
    /// 배달이 곧 복원 턴 끝의 증거다: 그 순간 최고치로 확정 · 배달 1초 안에 늦게 도착한 복원 끝 보고는 싣는다 · 그 뒤(배달이 부른
    /// 턴)는 싣지 않는다. 대기열이 아닌 바쁨 무시 입력(채널 행)은 복원 턴 도중에 와도 확정하지 않고, 상승 보고 전의 배달(대기열로
    /// 우회한 재주입 붙여넣기)도 확정 재료가 아니다.
    #[test]
    fn ctx_loop_guard_prompt_boundary_delivery_confirms_the_restore_end_without_a_gap() {
        let setup = || {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
            clear(&mut g, &mut armed, 100.0);
            g.note_quiescing(110.0);
            (g, armed)
        };
        let a = 110.0;
        // 틈 없는 인계(0.3초) + 제출(CR) 전에 늦게 도착한 복원 끝 보고(72) → 확정 72 · 그 뒤 배달 턴 76 은 작업.
        let (mut g, mut armed) = setup();
        g.note_delivery(Some(a - 50.0), Some(a - 49.6), a); // 지난 세션의 배달 — 재료 아님
        assert_eq!(report(&mut g, &mut armed, 64, 60, W200K, a + 5.0), None);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, a + 29.0), None);
        g.note_delivery(Some(a + 30.3), Some(a - 49.6), a + 30.3); // 인계 — 붙여넣기·CR 은 아직
        assert_eq!(g.restore_peak, Some(70), "틈 없는 프롬프트 경계 배달로 확정하지 않았다");
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 30.5), None); // CR 전에 늦게 도착한 복원 끝 보고
        g.note_delivery(Some(a + 30.3), Some(a + 30.7), a + 30.7); // 배달 제출(CR)
        assert_eq!(report(&mut g, &mut armed, 76, 60, W200K, a + 31.0), None); // 제출 뒤 보고 = 배달 메시지를 담음(1초 안이어도)
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, a + 40.0), None); // 배달이 부른 턴
        assert_eq!(g.restore_peak, Some(72), "CR 전 복원 끝 보고를 싣지 않았거나 제출 뒤(배달 메시지) 보고를 실었다");
        let b = g.bar(60, W200K).unwrap();
        assert_eq!((b.bar, b.regime, b.floor, b.settled), (80, CtxFloorRegime::Limited, 72, 77));
        // ★(샌드박스 반례 rv4-bl-1x22000 모양) 인계 1.5초 · 제출 0.4초 뒤 · 보고가 22KB 를 담아 0.7초 뒤 도착(77) — 복원 끝 72.
        let (mut g, mut armed) = setup();
        assert_eq!(report(&mut g, &mut armed, 63, 60, W200K, a + 0.5), None);
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 20.5), None);
        g.note_delivery(Some(a + 22.0), None, a + 22.0);
        g.note_delivery(Some(a + 22.0), Some(a + 22.4), a + 22.4);
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, a + 22.7), None);
        assert_eq!(g.restore_peak, Some(72), "제출된 배달을 담은 보고를 복원 끝 바닥에 실었다(참 바닥 72 → Stopped)");
        // 복원 턴 도중 채널 행(바쁨 무시 · 대기열 아님)은 확정하지 않는다 — 복원 끝 뒤 배달에서 확정.
        let (mut g, mut armed) = setup();
        assert_eq!(report(&mut g, &mut armed, 60, 60, W200K, a + 5.0), None);
        g.note_input(Some(a + 8.0), a + 8.0); // 채널 행
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, a + 20.0), None);
        assert_eq!(g.restore_peak, None, "복원 턴 도중의 채널 행으로 확정했다(복원 끝 과소)");
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, a + 30.0), None);
        g.note_delivery(Some(a + 30.4), None, a + 30.4);
        assert_eq!(g.restore_peak, Some(78));
        // 상승 보고 전의 배달(대기열로 우회한 재주입 붙여넣기)은 재료가 아니다.
        let (mut g, mut armed) = setup();
        g.note_delivery(Some(a + 1.0), Some(a + 1.4), a + 1.0);
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 20.0), None);
        assert_eq!(g.restore_peak, None, "재주입 붙여넣기 배달로 확정했다");
    }

    /// ★(R2NC5-1 (b) · R1-RUNAWAY-1 · RV2-ROLE-1 · ADV2-R1-1) Claude 자체 압축 재무장 세부 — 차단기가 섰던 좌석(복원 끝 신호 없이
    /// 정착 창이 배달을 흡수해 77 로 두 번 잼 → Stopped 87)은 Claude 압축(같은 세션 · 30%)을 보면 재무장하고 **압축 뒤 바닥을 다시
    /// 잰다** — 압축 관측부터 [`CTX_FLOOR_REREAD_SECS`](90초 · 연장 없음) 창의 최고치(30 · 창 밖 45 는 성장). 막대 = max(압축 뒤 막대
    /// 60, **clear 로 돌아오는 바닥 77 + 1**) = 78 — 그보다 낮은 발화는 clear 가 컨텍스트를 오히려 올린다(ADV2-R1-1: 참으로 가득 찬
    /// 좌석의 clear → 압축 → 재무장 고리). 78 에서 발화(`after_compaction` · 바닥 30) → 그 clear 가 바닥을 다시 잰다(복원 끝 72 ·
    /// 직전 세션이 차단기라 몰림 77 을 바닥으로 확정한 Backlog 82 · 그 발화가 일하는 좌석의 빠른 성장이면 다음 세션은 Limited 80).
    /// 압축 뒤 수준(지침 재읽기)이 기본 임계 위(63)여도 곧바로 발화하지 않는다. 창 안의 두 번째 압축(창 최고치보다 10%p 이상 낮음)은
    /// 다시 잰다(RV2-ROLE-1 (a)). 재무장하지 않는 경우: 낙폭 < 10%p · clear 뒤 정착 창 안 · 정상·Limited 세션(막대 ≤ 차단 상한) ·
    /// 창 전환(1M — 바닥을 토큰 비율로 옮겨 비교). 재무장한 세션은 발화 없는 교체(오너 수동 clear)도 다시 잰다.
    #[test]
    fn ctx_loop_guard_claude_compaction_rearms_a_stopped_seat_once() {
        let stopped_at_77 = || {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
            for (i, t) in [100.0, 1_000.0].into_iter().enumerate() {
                clear(&mut g, &mut armed, t);
                assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, t + 10.0), None);
                if i == 0 {
                    assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, t + 800.0), post(77, CtxFloorRegime::Probe));
                }
            }
            assert_eq!(g.effective_threshold(60, W200K, 2_000.0), 87, "전제: Stopped");
            (g, armed)
        };
        // 회복 경로.
        let (mut g, mut armed) = stopped_at_77();
        assert_eq!(report(&mut g, &mut armed, 84, 60, W200K, 2_100.0), None, "Stopped 87 아래");
        assert_eq!(g.observe(68, W200K, 2_150.0), None, "낙폭 9%p 는 압축이 아니다");
        let r = g.observe(30, W200K, 2_200.0).expect("Claude 압축(77 → 30)을 보지 못했다");
        assert_eq!((r.floor, r.regime), (77, CtxFloorRegime::Stopped));
        assert!(g.settling(2_210.0) && g.rearmed, "압축 뒤 바닥을 다시 재는 창이 열리지 않았다");
        assert_eq!(g.observe(25, W200K, 2_210.0), None, "압축 뒤 창 안의 작은 낙폭(5%p)을 압축으로 봤다");
        assert!(g.settling(2_200.0 + CTX_FLOOR_REREAD_SECS) && !g.settling(2_200.0 + CTX_FLOOR_REREAD_SECS + 1.0),
                "압축 뒤 재는 창이 재읽기 몫({CTX_FLOOR_REREAD_SECS}초)이 아니다(RV2-ROLE-1)");
        assert_eq!(g.effective_threshold(60, W200K, 2_220.0), 78, "(ADV2-R1-1) 막대가 clear 로 돌아오는 바닥 77 + 1 이 아니다");
        assert_eq!(report(&mut g, &mut armed, 45, 60, W200K, 2_300.0), None);
        assert_eq!(g.settle_peak, Some(30), "재읽기 창 밖(압축 100초 뒤) 성장이 압축 뒤 바닥에 들었다(RV2-ROLE-1)");
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, 3_900.0), None, "(ADV2-R1-1) clear 로 돌아오는 바닥(77)에서 발화했다");
        let rearmed = Some(CtxFire { floor: Some(30), regime: Some(CtxFloorRegime::Raise), settle_backstop: false, settled: Some(30),
                                     after_compaction: true });
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, 4_000.0), rearmed, "(②) 압축 뒤 바닥 위 성장에서 cys 사이클이 나지 않았다");
        // ★(R1-RUNAWAY-1) 압축 뒤 지침 재읽기로 기본 임계 위(63)에 서도 곧바로 발화하지 않는다 — 그 위로 자란 뒤(max(63 + 15 → 천장
        //   75, clear 로 돌아오는 바닥 77 + 1) = 78).
        let (mut g, mut armed) = stopped_at_77();
        assert!(g.observe(30, W200K, 2_200.0).is_some());
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 2_201.0), None);
        assert_eq!(report(&mut g, &mut armed, 63, 60, W200K, 2_220.0), None, "재읽기 턴(창 안)에서 발화했다");
        assert_eq!(report(&mut g, &mut armed, 63, 60, W200K, 3_000.0), None, "압축 뒤 수준 63 에서 성장 0 으로 발화했다(재읽기 고리 · ①)");
        assert_eq!(g.effective_threshold(60, W200K, 3_000.0), 78);
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, 5_000.0), None);
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, 5_100.0),
                   Some(CtxFire { floor: Some(63), regime: Some(CtxFloorRegime::Raise), settle_backstop: false, settled: Some(63),
                                  after_compaction: true }),
                   "(②) 압축 뒤 바닥 위로 자란 좌석을 clear 하지 않았다");
        // ★(RV2-ROLE-1 (a)) 압축 뒤 재는 창 안의 두 번째 압축(재읽기 63 → 25)도 압축이다 — 그 시각에서 다시 잰다(압축 전 최고치 63 을
        //   바닥으로 남기지 않는다) · clear 로 돌아오는 바닥은 처음 남긴 77 그대로.
        let (mut g, mut armed) = stopped_at_77();
        assert!(g.observe(30, W200K, 2_200.0).is_some());
        assert_eq!(report(&mut g, &mut armed, 63, 60, W200K, 2_230.0), None);
        let r2 = g.observe(25, W200K, 2_260.0).expect("창 안의 두 번째 압축(63 → 25)을 보지 못했다");
        assert_eq!(r2.floor, 63);
        assert_eq!((g.settle_anchor, g.settle_peak, g.rearm_floor), (Some(2_260.0), Some(25), Some(77)));
        assert_eq!(report(&mut g, &mut armed, 40, 60, W200K, 2_260.0 + CTX_FLOOR_REREAD_SECS + 10.0), None);
        assert_eq!(g.settle_peak, Some(25));
        // 다시 잰 바닥이 차단기 높이(78 · 압축이 덜 내렸다)면 다음 압축이 또 잰다 — 한 세션 여러 번(영구 Stopped 없음).
        let (mut g, mut armed) = stopped_at_77();
        assert_eq!(report(&mut g, &mut armed, 86, 60, W200K, 2_100.0), None);
        assert!(g.observe(66, W200K, 2_200.0).is_some(), "86 → 66 압축");
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, 2_220.0), None);
        assert_eq!(g.effective_threshold(60, W200K, 3_000.0), 88, "다시 잰 바닥 78(확인 유지) — Stopped");
        assert_eq!(report(&mut g, &mut armed, 86, 60, W200K, 3_100.0), None);
        assert!(g.observe(40, W200K, 3_200.0).is_some(), "두 번째 압축(86 → 40)을 다시 재지 않았다");
        // 그 clear — 복원 끝이 확정되는 운영 신호가 있으면 참 바닥으로 다시 잰다(차단기 해제).
        clear(&mut g, &mut armed, 4_100.0);
        g.note_quiescing(4_110.0);
        g.note_input(Some(4_110.0), 4_110.0);
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 4_140.0), None);
        g.note_delivery(Some(4_143.0), None, 4_143.0);
        g.note_idle(4_140.0, 4_143.0, 4_143.0);
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, 4_160.0), None);
        g.note_idle(4_160.0, 4_300.0, 4_300.0);
        assert!(!g.settling(4_300.0));
        // ★(RV2NC-E20-1) 직전 세션이 차단기(Stopped)였으므로 이 얇은 세션(복원 끝 72 · 몰림 77)은 몰림을 바닥으로 확정한다(Backlog
        //   83 — Stopped 87 아님 · 선제 압축점 아래). 그 발화가 일하는 좌석의 빠른 성장이면 확인을 풀어 다음 세션은 Limited 80.
        assert_eq!(g.effective_threshold(60, W200K, 4_300.0), 83, "회복하지 못했다(직전 Stopped 확인이 배달 흡수를 다시 Stopped 로)");
        assert_eq!(report(&mut g, &mut armed, 83, 60, W200K, 4_400.0), post_split(72, 77, CtxFloorRegime::Backlog));
        assert!(g.session_fast, "4%p 를 100초에 — 일하는 좌석의 빠른 성장");
        clear(&mut g, &mut armed, 4_500.0);
        g.note_quiescing(4_510.0);
        g.note_input(Some(4_510.0), 4_510.0);
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 4_540.0), None);
        g.note_delivery(Some(4_543.0), None, 4_543.0);
        g.note_idle(4_540.0, 4_543.0, 4_543.0);
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, 4_560.0), None);
        g.note_idle(4_560.0, 4_700.0, 4_700.0);
        assert_eq!(g.effective_threshold(60, W200K, 4_700.0), 80, "빠른 발화가 확인을 풀지 않았다");
        assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, 5_000.0), post_split(72, 77, CtxFloorRegime::Limited));
        // ★(RV2-ROLE-1 · ADV2-R1-1) 정착 창 안의 압축(차단기 세션 · 창 최고치보다 낙폭 이상 낮음)도 곧바로 재무장한다 — 창이 끝날
        //   때까지(최대 600초) 기다리면 그동안 자란 몫이 '압축 뒤 바닥'에 든다. 복원 끝 전(신호 없음)이고 최고치가 선제 압축 천장
        //   위였으면 사이클 자신의 입력이 부른 압축이다 — clear 로 돌아오는 바닥 = 선제 압축점(압축 뒤 cys 사이클 없음).
        let (mut g, mut armed) = stopped_at_77();
        clear(&mut g, &mut armed, 3_000.0);
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, 3_010.0), None);
        assert_eq!(g.observe(30, W200K, 3_020.0).map(|r| r.regime), Some(CtxFloorRegime::Stopped), "정착 창 안의 압축을 재무장하지 않았다");
        assert!(g.rearmed && g.settle_peak == Some(30) && g.rearm_floor == Some(83), "{g:?}");
        assert_eq!(g.effective_threshold(60, W200K, 3_500.0), CTX_FLOOR_NEVER, "clear 바닥이 압축점인 좌석의 압축 뒤 cys 사이클");
        // 낮은 높이(천장 아래)의 수동 /compact 는 사이클 자신의 입력이 부른 압축이 아니다 — 정상 세션은 재무장 없이 최고치를 싣는다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 64, 60, W200K, 110.0), None);
        assert_eq!(g.observe(40, W200K, 120.0), None, "정상 세션 창 안의 수동 /compact 를 재무장했다");
        assert_eq!(g.settle_peak, Some(64));
        // 정상·Limited 세션(막대 80 ≤ 차단 상한)은 재무장하지 않는다 — cys clear 가 선제 압축점 전에 난다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 110.0), None);
        assert_eq!(g.observe(30, W200K, 2_000.0), None, "Limited 세션을 재무장했다");
        assert_eq!(g.effective_threshold(60, W200K, 2_000.0), 80);
        // 창 전환(200K Stopped 77 → 1M 보고 15%)은 압축이 아니다 — 바닥을 1M 로 옮기면 16%.
        let (mut g, _) = stopped_at_77();
        assert_eq!(g.observe(15, W1M, 2_200.0), None, "창 전환을 압축으로 셌다");
        // 발화 없이 바뀐 새 세션(잰 바닥을 유지하는 교체 — Probe 좌석 · 수집기의 늦은 통지)의 낮은 첫 보고는 압축이 아니다
        // (handlers `context_threshold_idle_seat_with_a_floor_near_the_ceiling_does_not_refire` 의 모양 — 오판이면 유휴 좌석이
        // 기본 임계로 돌아가 붙여넣기 78 에서 재발화한다 · ①). 그 새 세션 안에서 다시 떨어지면 그때는 압축이다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, 110.0), None);
        assert_eq!(g.effective_threshold(60, W200K, 2_000.0), 80, "전제: Probe(창 밖)");
        clear(&mut g, &mut armed, 3_000.0); // 발화 없는 교체 — 잰 바닥 유지
        assert!(!g.settling(3_001.0));
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 3_001.0), None, "새 세션의 첫 보고(30)를 압축으로 봤다");
        assert!(!g.rearmed);
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, 3_010.0), None, "유휴 좌석이 붙여넣기 높이에서 재발화했다(①)");
        assert_eq!(report(&mut g, &mut armed, 79, 60, W200K, 3_500.0), None);
        assert!(g.observe(40, W200K, 4_000.0).is_some(), "같은 세션 안의 압축(79 → 40)을 보지 못했다");
        // 재무장 뒤 발화 없는 교체(오너 수동 clear)도 다시 잰다.
        let (mut g, mut armed) = stopped_at_77();
        assert!(g.observe(30, W200K, 2_200.0).is_some());
        clear(&mut g, &mut armed, 2_500.0);
        assert!(g.settling(2_510.0) && !g.rearmed, "재무장한 세션 뒤 교체를 다시 재지 않았다");
    }

    /// ★(자기 반례 탐색 · 운영 신호 모형 · 결정론 난수 400 좌석) 붙여넣기 0~90% × 복원 0~12%p·10~120초(가운데 멎음 20%) ×
    /// 사이클이 붙잡은 배달(0 또는 0.5~4%p · 1~3턴 · 조용함 3초마다) × 창(200K·1M·미상) × 기본 60·75 × 작업 0~3%p/분 ·
    /// 턴 10~50초 × 사이클 지연 20~120초 × Claude 선제 압축(창 − 33000), 3시간:
    /// (①) 유휴(작업 0)면 부트 1회 + 뒷문 1회 이하 · 일하면 사이클 수 ≤ 3 + 실제 성장(%p) + 압축 수 · Probe 는 연속 세션에 없다 ·
    ///     clear 뒤 발화는 뒷문·재무장·Stopped 가 아니면 보고 바닥 위 최소 여유(Probe 1%p) 이상 자란 뒤.
    /// (② · 오판) **참 바닥**(복원 끝 · 흡수분을 빼지 않는다) + 1 + 최소 여유 ≤ 차단 상한인 좌석은 차단기 영역(Probe·Stopped)에
    ///     가지 않고, 정착 창 최고치 + 1 ≤ 차단 상한이면 max(기본, 차단 상한) + 1 이하에서 clear · 보고 바닥 ≤ 참 바닥 + 1.
    /// (재무장) 재무장 발화는 압축 뒤에만 · 재무장 수 ≤ 압축 수.
    #[test]
    fn ctx_loop_guard_realistic_randomized_seats_keep_both_invariants() {
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut rnd = |lo: f64, hi: f64| {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            lo + (hi - lo) * ((state >> 11) as f64 / (1u64 << 53) as f64)
        };
        for case in 0..400 {
            let win = WINDOWS[(rnd(0.0, 3.0) as usize).min(2)];
            let base = if rnd(0.0, 1.0) < 0.8 { 60 } else { 75 };
            let paste = rnd(0.0, 90.0).round();
            let restore = if rnd(0.0, 1.0) < 0.2 { 0.0 } else { rnd(0.0, 12.0) };
            let restore_secs = rnd(10.0, 120.0);
            let stall = (rnd(0.0, 1.0) < 0.2).then(|| (rnd(1.0, restore_secs * 0.6), rnd(2.5, 6.0)));
            let (backlog, backlog_turns) = if rnd(0.0, 1.0) < 0.3 { (0.0, 0) } else { (rnd(0.5, 4.0), rnd(1.0, 3.99) as u32) };
            let work_per_min = if rnd(0.0, 1.0) < 0.3 { 0.0 } else { rnd(0.1, 3.0) };
            let queue_gap = if rnd(0.0, 1.0) < 0.5 { rnd(0.1, 1.9) } else { rnd(2.0, 10.0) };
            // 복원 턴 도중 바쁨을 보지 않는 입력(채널 행) — 멎음이 없으면 확정 재료가 아니다(멎음과 겹치는 드문 경우는 제외).
            let mid_input = (stall.is_none() && rnd(0.0, 1.0) < 0.3).then(|| rnd(0.5, restore_secs * 0.9));
            let seat = Seat { restore_secs, stall, backlog, backlog_turns, backlog_turn_secs: rnd(5.0, 20.0),
                              work_after: rnd(queue_gap, 60.0), work_per_min, turn_secs: rnd(10.0, 50.0),
                              cycle_delay: rnd(20.0, 120.0), quiesce: rnd(5.0, 30.0), queue_gap, mid_input,
                              ..Seat::real(base, win, paste, restore) };
            let run = run_seat(seat, 3.0 * 3600.0);
            let ctx = format!("case {case} {seat:?}: 발화 {:?} 압축 {:?}", run.fires, run.compactions);
            let h = ctx_floor_hard_cap(win);
            let restored = (paste + restore).min(100.0).round().max(3.0) as u16;
            assert!(run.rearms <= run.compactions.len(), "(재무장) 압축 없이 재무장 — {ctx}");
            if work_per_min == 0.0 {
                assert!(run.fires.len() <= 2 + usize::from(backlog > 0.0) + run.compactions.len(), "(①) 유휴 좌석 재발화 — {ctx}");
                continue;
            }
            let grown = work_per_min * 3.0 * 60.0;
            assert!((run.fires.len() as f64) <= 3.0 + grown + run.compactions.len() as f64, "(①) 사이클이 성장보다 많다 — {ctx}");
            let roomy = restored + 1 + u16::from(CTX_FLOOR_MIN_ROOM) <= u16::from(h);
            let mut prev_probe = false;
            for (i, (t, p, f, _)) in run.fires.iter().enumerate().skip(1) {
                if f.after_compaction {
                    let prev_t = run.fires[i - 1].0;
                    assert!(run.compactions.iter().any(|c| *c > prev_t && c <= t), "(재무장) 압축 없는 재무장 발화 — {ctx}");
                    prev_probe = false;
                    continue;
                }
                let probe = f.regime == Some(CtxFloorRegime::Probe) && !f.settle_backstop;
                assert!(!(prev_probe && probe), "(①) Probe 가 연속 세션에 — {ctx}");
                prev_probe = probe;
                let floor = u16::from(f.floor.expect("clear 뒤 발화의 잰 바닥"));
                let settled = u16::from(f.settled.expect("정착 창 최고치"));
                let min_room = if f.regime == Some(CtxFloorRegime::Probe) { 1 } else { u16::from(CTX_FLOOR_MIN_ROOM) };
                if !f.settle_backstop && f.regime != Some(CtxFloorRegime::Stopped) && u16::from(*p) < 100 {
                    assert!(u16::from(*p) >= floor + min_room.min(100 - floor), "(①) 바닥 {floor} 위 {p} — {ctx}");
                }
                if roomy {
                    assert!(matches!(f.regime, Some(CtxFloorRegime::Raise | CtxFloorRegime::Limited)), "(오판) 차단기 — {ctx}");
                    // 붙여넣기가 clear 직후(3%)보다 보이게 오르지 않으면 복원 턴의 보고(상승)가 없어 복원 끝을 확정할 수 없다 —
                    // 종전 판정(정착 창 최고치 · 최소 창 + 한 턴까지 흡수)으로 떨어진다(그 높이는 기본 임계 아래라 무해).
                    if restored > 4 {
                        assert!(floor <= restored + 1, "(오판) 보고 바닥 {floor} > 참 바닥 {restored} + 1 — {ctx}");
                    }
                    if settled + 1 <= u16::from(h) {
                        // 보고는 턴 끝마다 — 한 턴(≤ 1분)의 성장만큼 넘어 보일 수 있다(모형 해상도 · + 반올림 1).
                        let overshoot = work_per_min.ceil() as u16 + 1;
                        assert!(u16::from(*p) <= u16::from(base.max(h)) + overshoot, "(②) {p}% 까지 clear 가 늦었다 — {ctx}");
                    }
                }
            }
        }
    }

    /// ★(① 회귀 · 운영 신호 모형) RR3-R1-1·G3ROLE-1 의 유휴 불변식을 운영 신호 모형으로 다시 — 붙여넣기 0~100% × 복원(0·4·
    /// 8.45%p · 14·53·240초) × 창 × 기본 60·90, 유휴 2시간: 부트 1회(붙여넣기·복원만으로 max(기본, 차단 상한) 위면 뒷문 1회
    /// 더). 입력 기록·턴 끝 보고·1초 틱이 유휴 좌석을 다시 고리로 넣지 않는다.
    #[test]
    fn ctx_loop_guard_realistic_idle_seats_never_reenter_the_clear_loop() {
        for win in WINDOWS {
            for base in [60u8, 90] {
                for paste in (0..=100).step_by(4) {
                    for (restore, restore_secs) in [(0.0, 14.0), (4.0, 53.0), (8.45, 240.0)] {
                        let seat = Seat { restore_secs, ..Seat::real(base, win, f64::from(paste), restore) };
                        let run = run_seat(seat, 2.0 * 3600.0);
                        let floor = (f64::from(paste) + restore).min(100.0).round().max(3.0) as u8;
                        let expect = if floor >= base.max(ctx_floor_hard_cap(win)) { 2 } else { 1 };
                        assert!(run.fires.len() <= expect,
                                "창 {win:?} 기본 {base} 붙여넣기 {paste} 복원 +{restore}/{restore_secs}s: 유휴 좌석 발화 {:?}", run.fires);
                    }
                }
            }
        }
    }

    // ───────────── 가드 세부 ─────────────

    /// 200K master(run-m200 모양): 발화 → clear → 30% → 붙여넣기 67% 는 발화하지 않는다(실효 75 — 바닥+15=82 는 200K
    /// 창 천장 75 에 잘린다) → 75% 에서 한 번 발화 → 다음 clear 뒤 67% 는 조용하다(고리 없음).
    #[test]
    fn ctx_loop_guard_breaks_the_200k_master_reinject_loop() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE, "부트 뒤 첫 교차는 발화");
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 100.5), None);
        assert_eq!(report(&mut g, &mut armed, 67, 60, W200K, 106.0), None, "clear 직후 바닥에서 재발화");
        assert_eq!(g.effective_threshold(60, W200K, 106.0), 75, "200K 천장을 넘겨 올렸다");
        assert_eq!(report(&mut g, &mut armed, 74, 60, W200K, 2_000.0), None, "실효 임계 아래에서 발화");
        assert_eq!(report(&mut g, &mut armed, 75, 60, W200K, 2_010.0), post(67, CtxFloorRegime::Raise), "실효 임계에서 발화하지 않았다(② 무clear)");
        clear(&mut g, &mut armed, 2_100.0);
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 2_100.5), None);
        assert_eq!(report(&mut g, &mut armed, 67, 60, W200K, 2_106.0), None, "다음 사이클에 고리가 다시 섰다");
        assert_eq!(report(&mut g, &mut armed, 67, 60, W200K, 9_000.0), None, "정착 뒤 유휴 보고에서 발화");
    }

    /// ★(R2NC3-1 · ② 무clear) 200K CEO(run-ceo200 모양 · 바닥 69%): 실효 임계 75 + 사이클 여유가 Claude Code 선제 압축점
    /// (83.5%)·차단점(88.5%) 아래다(종전 84 는 압축점 위라 cys clear 가 영영 안 났다).
    #[test]
    fn ctx_loop_guard_200k_ceo_raise_stays_below_claude_code_limits() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 100.5), None);
        assert_eq!(report(&mut g, &mut armed, 69, 60, W200K, 106.0), None);
        let raised = g.effective_threshold(60, W200K, 106.0);
        assert_eq!(raised, 75, "200K CEO 가 천장 75 밖으로 올라갔다");
        let fire = max_tokens_shown_as(raised, 200_000);
        assert!(fire + CTX_FLOOR_CYCLE_MARGIN_TOKENS <= 167_000,
                "올린 임계({raised}%) + 사이클 여유가 Claude Code 선제 압축점(167000 = 83.5%)을 넘는다 — cys clear 가 안 난다");
        assert!(fire + CTX_FLOOR_CYCLE_MARGIN_TOKENS < 177_000, "올린 임계({raised}%) + 사이클 여유가 차단점(177000 = 88.5%)에 닿는다");
        assert_eq!(report(&mut g, &mut armed, 75, 60, W200K, 2_000.0), post(69, CtxFloorRegime::Raise), "올린 임계에서 발화하지 않았다(② 무clear)");
    }

    /// ★(RR3-R1-1) 천장 아래 여유가 최소 여유보다 좁은 바닥(200K 71~75 · 1M 81~90)은 **바닥에서 발화하지 않고** 차단
    /// 상한(200K 80 · 1M 95)에서 clear 한다(Limited) — 종전(52b8c656)은 그 바닥에서 곧바로 발화해 유휴 좌석이 고리를 돌았다.
    #[test]
    fn ctx_loop_guard_floor_just_below_the_ceiling_waits_for_the_hard_cap() {
        for (floor, win, cap) in [(71u8, W200K, 80u8), (72, W200K, 80), (74, W200K, 80), (75, W200K, 80), (72, None, 80),
                                  (81, W1M, 95), (90, W1M, 95)] {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 61, 60, win, 0.0), FIRE);
            clear(&mut g, &mut armed, 100.0);
            assert_eq!(report(&mut g, &mut armed, floor, 60, win, 110.0), None, "바닥 {floor}({win:?}) 에서 재발화(① 고리)");
            assert_eq!(report(&mut g, &mut armed, floor, 60, win, 5_000.0), None, "바닥 {floor}({win:?}) 유휴 보고에서 발화");
            assert_eq!(report(&mut g, &mut armed, cap - 1, 60, win, 5_010.0), None);
            assert_eq!(report(&mut g, &mut armed, cap, 60, win, 5_020.0), post(floor, CtxFloorRegime::Limited),
                       "바닥 {floor}({win:?}) 가 차단 상한 {cap} 에서 clear 되지 않았다(② 무clear)");
        }
    }

    /// ★(RR3-R1-1 · 연속 clear 차단기) 차단 상한 근처 바닥(200K 76~): 첫 세션은 **1회만** 차단 상한에서 재시도(Probe ·
    /// 바닥에서 1%p 이상 자란 뒤), 다음 clear 뒤에도 그러면 자동 clear 중단(Stopped) — 바닥+10 까지 실제로 자랄 때만 발화.
    /// 오너 고지: Probe(warn · Limited 계수 1) → Stopped(error · 1·2·4…번째) — 무한 재시도·무음 둘 다 금지.
    #[test]
    fn ctx_loop_guard_breaker_retries_once_then_stops_until_real_growth() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 100.5), None);
        g.observe(78, W200K, 110.0);
        assert_eq!(g.effective_threshold(60, W200K, 110.0), 80);
        let n = g.hold_notice(78, 60, W200K, 110.0).expect("Probe 이벤트");
        assert_eq!((n.regime, n.bar, n.feed, n.event), (CtxFloorRegime::Probe, 80, None, true), "복원 도중(정착 창 안)에 오너 feed");
        let n = g.hold_notice(78, 60, W200K, 2_000.0).expect("정착 창이 닫힌 뒤 오너 고지");
        assert_eq!((n.regime, n.bar, n.feed, n.count, n.event), (CtxFloorRegime::Probe, 80, Some("warn"), 1, false));
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, 3_000.0), None, "유휴 Probe 좌석이 발화했다");
        assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, 3_010.0), post(78, CtxFloorRegime::Probe), "1회 재시도가 없다");
        // 재시도 뒤 같은 바닥 — 확인 → 자동 clear 중단(정착 창이 닫힌 뒤 바닥+10).
        clear(&mut g, &mut armed, 3_100.0);
        g.observe(78, W200K, 3_110.0);
        assert_eq!(g.effective_threshold(60, W200K, 3_110.0), 80,
                   "정착 창 안 뒷문(재시도가 창 밖 발화라 다시 무장)이 차단 상한에 없다 — 창 안 빠른 실제 성장(②)");
        g.observe(78, W200K, 3_110.0 + CTX_FLOOR_SETTLE_QUIET_SECS);
        assert_eq!(g.effective_threshold(60, W200K, 3_500.0), 88, "확인된 바닥에서 자동 clear 가 멈추지 않았다");
        let n = g.hold_notice(78, 60, W200K, 3_500.0).expect("Stopped 고지");
        assert_eq!((n.regime, n.bar, n.feed, n.count), (CtxFloorRegime::Stopped, 88, Some("error"), 1));
        for (i, p) in [80u8, 84, 87].into_iter().enumerate() {
            assert_eq!(report(&mut g, &mut armed, p, 60, W200K, 4_000.0 + i as f64), None, "Stopped 좌석이 {p}% 에서 발화");
        }
        assert_eq!(report(&mut g, &mut armed, 88, 60, W200K, 4_100.0), post(78, CtxFloorRegime::Stopped),
                   "실제로 바닥+10 자란 좌석을 clear 하지 않았다(②)");
        // 다음 세션도 그대로면 곧바로 Stopped(재시도 없음) · 오너 고지는 2·4번째.
        let mut feeds = vec![];
        for k in 0..6 {
            let t = 5_000.0 + 1_000.0 * k as f64;
            clear(&mut g, &mut armed, t);
            g.observe(78, W200K, t + 5.0);
            g.observe(78, W200K, t + 200.0);
            assert_eq!(g.effective_threshold(60, W200K, t + 500.0), 88);
            feeds.push(g.hold_notice(78, 60, W200K, t + 500.0).and_then(|n| n.feed));
            assert_eq!(report(&mut g, &mut armed, 88, 60, W200K, t + 900.0).map(|f| f.regime), Some(Some(CtxFloorRegime::Stopped)));
        }
        assert_eq!(feeds, vec![Some("error"), None, Some("error"), None, None, None], "Stopped 고지 주기(2·4·8…)");
    }

    /// 자동 clear 를 멈춘(Stopped) 좌석은 오너의 수동 clear·재기동(발화 없는 세션 교체)도 다시 잰다 — 처방(지침 축소) 뒤
    /// 바닥이 내려가면 곧바로 정상 영역. 멈추지 않은 좌석은 발화 없는 교체를 무시한다(종전).
    #[test]
    fn ctx_loop_guard_stopped_seat_remeasures_on_any_new_session() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        for (t, _) in [(100.0, 0), (1_000.0, 1)] {
            clear(&mut g, &mut armed, t);
            report(&mut g, &mut armed, 78, 60, W200K, t + 10.0);
            let _ = report(&mut g, &mut armed, 80, 60, W200K, t + 800.0);
        }
        assert_eq!(g.effective_threshold(60, W200K, 1_900.0), 88, "전제: Stopped");
        clear(&mut g, &mut armed, 3_000.0); // 발화 없는 교체(오너 수동 clear)
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 3_010.0), None);
        assert_eq!(g.effective_threshold(60, W200K, 3_010.0), 60, "처방 뒤 새 바닥(30)을 다시 재지 않았다");
        assert_eq!(report(&mut g, &mut armed, 60, 60, W200K, 9_000.0), post(30, CtxFloorRegime::Raise));
        // 멈추지 않은 좌석(Raise)은 발화 없는 두 번째 교체를 무시한다 — 세션 흔들림이 정착 창을 다시 열지 않는다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        report(&mut g, &mut armed, 67, 60, W200K, 110.0);
        clear(&mut g, &mut armed, 2_000.0);
        assert!(!g.settling(2_010.0), "발화 없는 교체가 정착 창을 다시 열었다(흔들림이 발화를 막는다 — ②)");
        assert_eq!(report(&mut g, &mut armed, 75, 60, W200K, 2_020.0), post(67, CtxFloorRegime::Raise));
    }

    /// 억제 직후 다음 보고가 실효 임계를 **한 번에 건너뛰어도**(큰 파일 읽기 한 번 · 67% → 90%) 발화한다 — 보류가
    /// 래치를 소진했다면 그 세션 내내 발화하지 않는다(② 무clear).
    #[test]
    fn ctx_loop_guard_hold_keeps_the_latch_armed() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 67, 60, W200K, 106.0), None);
        assert!(armed, "보류가 래치를 소진했다");
        assert_eq!(report(&mut g, &mut armed, 90, 60, W200K, 1_000.0), post(67, CtxFloorRegime::Raise),
                   "실효 임계를 건너뛴 보고에서 발화하지 않았다");
    }

    /// 1M 창(오너 좌석): clear 뒤 바닥 13% — 60% 교차는 종전대로 발화(정책 무변화 · 보류 고지 없음).
    #[test]
    fn ctx_loop_guard_leaves_1m_seats_on_the_60_percent_policy() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 60, 60, W1M, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 13, 60, W1M, 160.0), None);
        assert_eq!(report(&mut g, &mut armed, 15, 60, W1M, 350.0), None);
        assert_eq!(g.hold_notice(59, 60, W1M, 400.0), None);
        assert_eq!(report(&mut g, &mut armed, 60, 60, W1M, 9_000.0), post(15, CtxFloorRegime::Raise), "1M 좌석의 60% clear 가 막혔다");
    }

    /// 200K 좌석이라도 바닥이 낮고(30%) 일해서 60% 에 닿았으면 종전대로 발화한다(오탐 금지) — 정착 창 안의 빠른 성장은
    /// 바닥에 들어가도(45) 실효 임계는 기본 60 이다.
    #[test]
    fn ctx_loop_guard_fires_when_the_seat_grew_by_real_work() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 30, 60, W200K, 150.0), None);
        assert_eq!(report(&mut g, &mut armed, 45, 60, W200K, 390.0), None, "정착 창 안의 빠른 성장");
        assert_eq!(report(&mut g, &mut armed, 60, 60, W200K, 900.0), post(45, CtxFloorRegime::Raise), "일해서 찬 교차가 보류됐다");
    }

    /// 세션 교체가 없는 재교차(자기보고 흔들림 59↔61)는 가드 대상이 아니다 — 종전대로 발화 · 발화 이력 없는 좌석의
    /// 세션 교체도 무시(부트·phoenix --resume 첫 교차는 종전대로 발화 = 실패 방향).
    #[test]
    fn ctx_loop_guard_ignores_recross_without_a_session_change() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, None, 0.0), FIRE);
        assert_eq!(report(&mut g, &mut armed, 59, 60, None, 10.0), None);
        assert_eq!(report(&mut g, &mut armed, 61, 60, None, 20.0), FIRE);
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        g.note_session_change(5.0);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W1M, 6.0), FIRE, "발화 이력 없는 첫 교차가 보류됐다");
    }

    /// 재료 결측 = 종전대로 발화: 정착 창 안 관측이 하나도 없으면(교체 뒤 첫 보고가 창 밖) 바닥 미상 → 기본 임계 ·
    /// 역할 override 가 천장 이상(90)이면 그 임계가 우선(바닥 91 은 Probe — 91 에선 발화 없음 · 92 에서).
    #[test]
    fn ctx_loop_guard_missing_floor_fails_toward_firing() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 67, 60, W200K, 100.0 + CTX_FLOOR_SETTLE_MAX_SECS + 1.0), FIRE);
        // 붙여넣기만으로 뒷문 높이(max(기본 90, 차단 상한 80)) 위인 좌석: 뒷문 1회 → 다음 세션은 뒷문 없이 증명 → Stopped.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 91, 90, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 91, 90, W200K, 110.0), backstop(91, CtxFloorRegime::Probe), "뒷문 1회");
        clear(&mut g, &mut armed, 200.0);
        for t in [210.0, 240.0, 400.0, 5_000.0] {
            assert_eq!(report(&mut g, &mut armed, 91, 90, W200K, t), None, "바닥 91 에서 뒷문을 연속으로 썼다(① 고리) t={t}");
        }
        assert_eq!(g.effective_threshold(90, W200K, 5_000.0), CTX_FLOOR_NEVER, "증명·확인된 바닥 91 — 자동 clear 중단");
    }

    /// 정착 창: 기본 300초 · 바닥이 최근 120초 안에 올랐으면 연장(복원이 도는 중) · 최대 600초(연장이 발화를 영영 막지
    /// 않는다 — ②) · quiescing(사이클 진행 중) 관측이 시작점을 뒤로 민다(교체 + 300초까지만 — 멈춘 quiescing 무한 연장 금지).
    #[test]
    fn ctx_loop_guard_settle_window_follows_the_restore_but_is_bounded() {
        // 느린 복원(30초마다 +1%p · 기본 창 300초를 넘는 8분) — 전부 바닥이다 · 멎고 120초 뒤 닫힌다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        for k in 0..=16u8 {
            assert_eq!(report(&mut g, &mut armed, 63 + k / 2, 60, W200K, 100.0 + 30.0 * f64::from(k)), None);
        }
        let last = 100.0 + 30.0 * 16.0; // 580 — 기본 창(400) 밖
        assert_eq!(g.settle_peak, Some(71), "기본 창 밖으로 이어진 복원이 바닥에 들지 않았다");
        assert!(g.settling(last + 100.0), "복원이 도는데 창이 닫혔다");
        let (mut g2, mut armed2) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g2, &mut armed2, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g2, &mut armed2, 100.0);
        for k in 0..=8u8 {
            let _ = report(&mut g2, &mut armed2, 63 + k, 60, W200K, 100.0 + 30.0 * f64::from(k)); // 마지막 상승 340
        }
        assert!(g2.settling(340.0 + CTX_FLOOR_SETTLE_QUIET_SECS), "기본 창 밖 복원 연장이 없다");
        assert!(!g2.settling(340.0 + CTX_FLOOR_SETTLE_QUIET_SECS + 1.0), "복원이 멎은 뒤에도 창이 열려 있다");
        // 끝없이 오르는 좌석 — 600초에서 닫힌다(그 뒤 성장은 작업).
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W1M, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        for k in 0..100u32 {
            let _ = report(&mut g, &mut armed, (10 + k).min(100) as u8, 60, W1M, 100.0 + 20.0 * f64::from(k));
        }
        assert!(!g.settling(100.0 + CTX_FLOOR_SETTLE_MAX_SECS + 1.0));
        assert!(g.settle_peak.unwrap() <= 10 + (CTX_FLOOR_SETTLE_MAX_SECS / 20.0) as u8 + 1);
        // quiescing 200초(붙여넣기 대기) 뒤 복원 250초 — 시작점이 밀려 복원 전체가 바닥이다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        for k in 0..=20 {
            g.note_quiescing(100.0 + 10.0 * f64::from(k));
            let _ = report(&mut g, &mut armed, 3, 60, W200K, 100.0 + 10.0 * f64::from(k));
        }
        for k in 0..=10u8 {
            let _ = report(&mut g, &mut armed, 60 + k, 60, W200K, 300.0 + 25.0 * f64::from(k));
        }
        assert_eq!(g.settle_peak, Some(70), "quiescing 뒤 복원이 바닥에 들지 않았다");
        // 멈춘 quiescing — 시작점은 교체 + 300초 넘어 밀리지 않는다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        for k in 0..400 {
            g.note_quiescing(100.0 + 10.0 * f64::from(k));
        }
        assert!(g.settle_anchor.unwrap() <= 100.0 + CTX_FLOOR_SETTLE_SECS);
        assert!(!g.settling(100.0 + 2.0 * CTX_FLOOR_SETTLE_SECS + CTX_FLOOR_SETTLE_MAX_SECS));
    }

    /// 창이 바뀌면(200K → 1M 전환) 잰 바닥을 토큰 비율로 옮긴다 — 72%(144K) 는 1M 에서 15% → 기본 60 정책.
    /// 창 미상 보고는 바닥의 창을 유지한다(결측은 값이 아니다).
    #[test]
    fn ctx_loop_guard_moves_the_floor_when_the_window_changes() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, 110.0), None);
        assert_eq!(g.effective_threshold(60, None, 5_000.0), 80, "창 미상 보고가 바닥의 창을 버렸다");
        assert_eq!(g.effective_threshold(60, W200K, 5_000.0), 80);
        assert_eq!(g.effective_threshold(60, W1M, 5_000.0), 60, "1M 전환 뒤에도 200K 바닥으로 올린 임계가 남았다");
        assert_eq!(report(&mut g, &mut armed, 60, 60, W1M, 5_000.0), post(15, CtxFloorRegime::Raise));
    }

    /// 고지 — 이벤트는 같은 세션 안에서 영역이 올라갈 때만(사이클마다 같은 (영역, 임계)는 이벤트 없음) · 오너 feed 는
    /// 정착 창이 닫힌 뒤(복원 도중의 바닥으로 말하지 않는다) 세션당 1번 · Raise feed 는 좌석당 1회.
    #[test]
    fn ctx_loop_guard_hold_notices_are_not_repeated_every_cycle() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 61, 60, W200K, 0.0), FIRE);
        let (mut events, mut feeds) = (vec![], vec![]);
        for k in 0..3 {
            let t = 100.0 + 3_000.0 * f64::from(k);
            clear(&mut g, &mut armed, t);
            g.observe(67, W200K, t + 5.0);
            let _ = g.effective_threshold(60, W200K, t + 5.0);
            events.push(g.hold_notice(67, 60, W200K, t + 5.0).map(|n| (n.event, n.feed)));
            assert_eq!(g.hold_notice(68, 60, W200K, t + 6.0), None, "같은 세션·같은 영역 이벤트가 반복됐다");
            feeds.push(g.hold_notice(68, 60, W200K, t + 1_000.0).and_then(|n| n.feed));
            assert_eq!(g.hold_notice(69, 60, W200K, t + 1_100.0), None, "같은 세션의 오너 feed 가 반복됐다");
            assert_eq!(report(&mut g, &mut armed, 75, 60, W200K, t + 2_000.0).map(|f| f.regime), Some(Some(CtxFloorRegime::Raise)));
        }
        assert_eq!(events, vec![Some((true, None)), None, None], "정착 창 안 오너 feed · 사이클마다 같은 이벤트");
        assert_eq!(feeds, vec![Some("warn"), None, None], "Raise 오너 feed 는 좌석당 1회");
        // 기본 임계 아래 보고는 고지하지 않는다.
        clear(&mut g, &mut armed, 20_000.0);
        g.observe(50, W200K, 20_005.0);
        assert_eq!(g.hold_notice(50, 60, W200K, 21_000.0), None);
    }

    /// 세션 교체 판정 — 줄기(세션 id)가 다를 때만 · 결측·표기 차이는 교체가 아니다.
    #[test]
    fn session_file_changed_compares_session_stems_only() {
        assert!(session_file_changed("/p/proj/a.jsonl", "/p/proj/b.jsonl"));
        assert!(!session_file_changed("/p/proj/a.jsonl", "/private/p/proj/a.jsonl"), "표기 차이를 교체로 셌다");
        assert!(!session_file_changed("", "/p/proj/b.jsonl"), "결측을 교체로 셌다");
        assert!(!session_file_changed("/p/proj/a.jsonl", ""), "결측을 교체로 셌다");
        assert!(!session_file_changed("/p/proj/a.jsonl", "/p/proj/a.jsonl"));
    }

    /// 배선 핀: 두 관측 경로(transcript 수집기 · statusline 보고)가 세션 교체를 가드에 알리고, 발화 판정은 가드를
    /// 거친다(관측 → 실효 임계 → 보류 고지 · 교차) — 한 곳이라도 빠지면 그 경로의 좌석에서 고리가 되살아난다.
    #[test]
    fn ctx_loop_guard_is_wired_into_both_observation_paths_and_the_gate() {
        let usage = include_str!("usage.rs");
        let collect = &usage[usage.find("fn collect_for(").expect("collect_for")..];
        let collect = &collect[..collect.find("\n}\n").expect("collect_for 끝")];
        assert!(collect.contains("session_file_changed(") && collect.contains(".note_session_change("),
                "transcript 수집기의 세션 교체가 가드에 닿지 않는다");
        let handlers = include_str!("handlers.rs");
        let report = &handlers[handlers.find("\"usage.report\" =>").expect("usage.report")..];
        let report = &report[..report.find("\"usage.report_account\" =>").expect("다음 팔")];
        let note = report.find(".note_session_change(").expect("statusline 경로가 세션 교체를 가드에 알리지 않는다");
        let fire = report.find("maybe_fire_context_threshold(").expect("statusline 발화");
        assert!(note < fire, "세션 교체 통지가 발화 판정보다 뒤다(교차 보고에서 가드가 늦는다)");
        let wrapper = &handlers[handlers.find("pub(crate) fn maybe_fire_context_threshold(").expect("gate")..];
        let wrapper = &wrapper[..wrapper.find("\n}\n").expect("gate 끝")];
        assert!(wrapper.contains("maybe_fire_context_threshold_at("), "운영 경로가 본체를 거치지 않는다");
        let gate = &handlers[handlers.find("pub(crate) fn maybe_fire_context_threshold_at(").expect("gate 본체")..];
        let gate = &gate[..gate.find("\n}\n").expect("gate 끝")];
        for needle in [".note_quiescing(", ".observe(", ".effective_threshold(", ".hold_notice(", ".on_crossing("] {
            assert!(gate.contains(needle), "발화 판정이 가드의 {needle} 를 거치지 않는다");
        }
        // 조용함 닫힘: 끝난 틈은 관측 전 · 이어지는 조용함은 관측 뒤(순서가 바뀌면 틈 뒤 작업이 바닥에 들거나 복원 끝이 빠진다).
        let (gap, obs) = (gate.find("idle.last_gap").expect("게이트가 끝난 틈을 보지 않는다"), gate.find(".observe(").unwrap());
        let ongoing = gate.find(".note_idle(idle.quiet_since").expect("게이트가 이어지는 조용함을 보지 않는다");
        assert!(gap < obs && obs < ongoing, "조용함 닫힘 순서(틈 → 관측 → 이어지는 조용함)가 어긋났다");
        assert!(wrapper.contains("CtxIdleObs::of("), "운영 경로가 좌석의 조용함 관측을 넘기지 않는다");
        // ★(ROLE-R4-1 · R2NC5-1) 좌석 입력 시각은 틈·관측 **전** — 복원 턴 뒤 첫 입력(붙잡혔던 배달)이 복원 끝 바닥을 확정한다.
        let input = gate.find(".note_input(idle.last_input").expect("게이트가 좌석 입력 시각을 가드에 넘기지 않는다");
        assert!(input < gap && input < obs, "좌석 입력 → 틈 → 관측 순서가 어긋났다(배달이 부른 턴이 복원 끝 바닥에 든다)");
        let deliv = gate.find(".note_delivery(idle.last_queue_delivery, idle.inject_done").expect("게이트가 대기열 배달·제출 시각을 가드에 넘기지 않는다");
        assert!(deliv < gap && deliv < obs, "대기열 배달 → 틈 → 관측 순서가 어긋났다");
        // ★(R2NC6-1) 대기열에 남은 항목(붙잡혔던 몰림이 끝났나)도 조용함 판정 전에.
        let queue = gate.find(".note_queue(idle.queue_oldest)").expect("게이트가 대기열 잔여를 가드에 넘기지 않는다");
        assert!(queue < gap && queue < obs, "대기열 잔여 → 틈 → 관측 순서가 어긋났다");
        assert!(gate.contains("\"context.floor_rearmed\""), "Claude 압축 재무장이 관측되지 않는다");
        let of = &usage[usage.find("pub fn of(daemon: &Daemon, s: &Surface)").expect("CtxIdleObs::of")..];
        assert!(of[..of.find("\n    }\n").unwrap()].contains("last_input_of(s)"), "조용함 관측이 좌석 입력 시각을 싣지 않는다");
        assert!(of[..of.find("\n    }\n").unwrap()].contains("last_queue_delivery_at"), "조용함 관측이 대기열 배달 시각을 싣지 않는다");
        assert!(of[..of.find("\n    }\n").unwrap()].contains("inject_track.done_at()"), "조용함 관측이 배달 제출(CR) 시각을 싣지 않는다");
        assert!(of[..of.find("\n    }\n").unwrap()].contains("pending_queue"), "조용함 관측이 대기열 잔여(enqueue 시각)를 싣지 않는다");
        let lio = &usage[usage.find("fn last_input_of(").expect("last_input_of")..];
        let lio = &lio[..lio.find("\n}\n").unwrap()];
        for needle in ["inject_track.done_at()", "last_injected", "last_human_input"] {
            assert!(lio.contains(needle), "좌석 입력 시각이 {needle} 를 보지 않는다");
        }
        // 수집기 틱이 보고 없는 유휴 좌석도 닫는다 · PTY reader 가 끝난 틈을 기록한다.
        let tick = &usage[usage.find("fn collect_tick(").expect("collect_tick")..];
        let tick = &tick[..tick.find("\n}\n").expect("collect_tick 끝")];
        assert!(tick.contains("ctx_guard_tick("), "수집기 틱이 가드를 두드리지 않는다(유휴 좌석의 정착 창이 조용함으로 닫히지 않는다)");
        let tick_at = &usage[usage.find("pub(crate) fn ctx_guard_tick_at(").expect("ctx_guard_tick_at")..];
        let tick_at = &tick_at[..tick_at.find("\n}\n").unwrap()];
        assert!(tick_at.find(".note_input(").is_some_and(|i| i < tick_at.find(".note_idle(").unwrap()),
                "수집기 틱이 좌석 입력 시각을 조용함 판정 전에 넘기지 않는다");
        assert!(tick_at.find(".note_delivery(").is_some_and(|i| i < tick_at.find(".note_idle(").unwrap()),
                "수집기 틱이 대기열 배달 시각을 조용함 판정 전에 넘기지 않는다");
        assert!(tick_at.find(".note_queue(").is_some_and(|i| i < tick_at.find(".note_idle(").unwrap()),
                "수집기 틱이 대기열 잔여를 조용함 판정 전에 넘기지 않는다");
        let state = include_str!("state.rs");
        assert!(state.contains("last_output_gap.lock()") && state.contains("CTX_FLOOR_IDLE_QUIET_SECS"),
                "PTY reader 가 끝난 조용한 틈을 기록하지 않는다");
        assert!(state.contains("pub(crate) fn done_at(&self) -> Option<Instant>"), "writer Inject 끝 시각을 읽을 수 없다");
    }

    // ───────────── ★(R2NC6-1 · RV-ROLE-CF42-1 · R1-RUNAWAY-1) 재검증 2회차 반례 — 바쁜 여유 좌석 · 압축 뒤 재읽기 고리 ─────────────

    /// 운영 신호 모형의 바쁜 좌석 — 복원 턴 30초 뒤 5초 쉬고 분당 `rate`%p(매 분 `turn`초 턴 · 턴 사이 틈 = 60 − turn 초 ·
    /// 턴 시작마다 대기열 배달), 사이클이 붙잡았던 배달 `backlog`%p/`turns`턴.
    fn busy_real(win: Option<u64>, paste: f64, restore: f64, rate: f64, turn: f64, backlog: f64, turns: u32) -> Seat {
        Seat { restore_secs: 30.0, work_after: 5.0, work_per_min: rate, turn_secs: turn, backlog, backlog_turns: turns,
               ..Seat::real(60, win, paste, restore) }
    }

    /// ★(R2NC6-1 · RV-ROLE-CF42-1 · ②) **바쁜 여유 좌석**(참 바닥 72 master · 56 worker · 200K·창 미상)은 턴 사이 틈이 2~15초뿐
    /// 이어도(대기열 최소 간격 10초 · 프롬프트 경계 배달은 조용함을 기다리지 않는다 — 15초 조용함이 오지 않는다) Claude 선제
    /// 압축(83.5%)보다 먼저 cys clear 된다: 압축 0 · clear 뒤 발화는 Raise·Limited · max(기본, 차단 상한) + 1 이하 · 좌석이
    /// 선제 압축점 근처로 끌려가지 않는다. d9638152 는 복원 끝이 확정된 뒤 정착 창을 15초 조용함에서만 닫아, 창이 시작점 +
    /// 300~600초까지 열린 채 작업을 최고치에 싣고(막대 = 최고치 + 1 → 82~86) 뒷문은 첫 세션에만 있어 둘째 세션부터 매번 Claude
    /// 압축에 clear 를 빼앗겼다(재검증자 run_seat 반례 · 샌드박스 드릴 run-rvnc6-*). 틈 20초(15초 조용함이 온다)는 대조다.
    #[test]
    fn r2nc6_busy_roomy_seat_with_short_gaps_is_cleared_before_claude_compaction() {
        let mut bad = vec![];
        let mut check = |name: &str, seat: Seat, cap: u8| {
            let run = run_seat(seat, 3.0 * 3600.0);
            let post: Vec<_> = run.fires.iter().skip(1).map(|(t, p, f, _)| (*t as u32, *p, f.regime, f.floor, f.settled, f.settle_backstop)).collect();
            let line = format!("{name} 발화 {} 압축 {:?} 최고 {} {post:?}", run.fires.len(), run.compactions, run.max_post_clear);
            let late = run.fires.iter().skip(1).any(|(_, p, f, _)| {
                *p > cap + 1 || f.after_compaction || !matches!(f.regime, Some(CtxFloorRegime::Raise | CtxFloorRegime::Limited))
            });
            if run.fires.len() < 3 || !run.compactions.is_empty() || late || run.max_post_clear >= 83 {
                bad.push(line);
            }
        };
        for win in [W200K, None] {
            // 적체 없음 · 분당 1.0~3.0%p × 틈 3·8·14초(턴 57·52·46초) — 재검증자 반례 격자 + 대조 틈 20초.
            for rate in [1.0, 1.3, 1.6, 2.0, 2.5, 3.0] {
                for turn in [40.0, 46.0, 52.0, 57.0] {
                    check(&format!("{win:?} master 72 분당 {rate} 틈 {}s", 60.0 - turn), busy_real(win, 63.3, 8.9, rate, turn, 0.0, 0), 80);
                    // 참 바닥 56 worker(Raise — 천장 75).
                    check(&format!("{win:?} worker 56 분당 {rate} 틈 {}s", 60.0 - turn), busy_real(win, 51.0, 5.0, rate, turn, 0.0, 0), 75);
                }
            }
            // 사이클이 붙잡았던 배달 5%p(3턴) + 분당 0.8~1.3%p(RV-ROLE-CF42-1 '적체 5%p 를 더하면 분당 0.8%p 만으로도 압축 7').
            for rate in [0.8, 1.0, 1.3] {
                for turn in [46.0, 52.0, 57.0] {
                    check(&format!("{win:?} master 72 적체 5 분당 {rate} 틈 {}s", 60.0 - turn), busy_real(win, 63.3, 8.9, rate, turn, 5.0, 3), 80);
                }
            }
        }
        assert!(bad.is_empty(), "(②) 바쁜 여유 좌석의 cys clear 가 늦었다(Claude 선제 압축 83.5% 가 먼저 오거나 차단 상한 위) — {} 건:\n{}",
                bad.len(), bad.join("\n"));
    }

    /// ★(R2NC6-1 · ②) 같은 바쁜 여유 좌석에서 **자동 압축을 끈 좌석**(모형은 차단점 88.5% 위로도 자란다 — 실좌석은 거기서 프롬프트가
    /// 멈춘다)은 차단 상한(80) + 1 이하에서 clear 되고 차단점 근처로 끌려가지 않는다. d9638152 는 88~100% 에서 발화했다(드릴
    /// run-rvnc6-*-noac · 오너 feed '93% 에서 clear').
    #[test]
    fn r2nc6_busy_roomy_seat_without_autocompact_is_cleared_below_the_blocking_point() {
        let mut bad = vec![];
        for win in [W200K, None] {
            for rate in [1.3, 1.6, 2.0, 2.5, 3.0] {
                for turn in [46.0, 52.0, 57.0] {
                    let seat = Seat { compact_at: None, ..busy_real(win, 63.3, 8.9, rate, turn, 0.0, 0) };
                    let run = run_seat(seat, 3.0 * 3600.0);
                    let post: Vec<_> = run.fires.iter().skip(1).map(|(t, p, f, _)| (*t as u32, *p, f.regime, f.settled)).collect();
                    if run.fires.len() < 3 || run.fires.iter().skip(1).any(|(_, p, _, _)| *p > 81) || run.max_post_clear >= 88 {
                        bad.push(format!("{win:?} 분당 {rate} 틈 {}s 최고 {} {post:?}", 60.0 - turn, run.max_post_clear));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "(②) 자동 압축을 끈 바쁜 여유 좌석이 차단점(88.5%) 근처까지 cys clear 없이 자랐다 — {} 건:\n{}",
                bad.len(), bad.join("\n"));
    }

    /// ★(R2NC6-1 · RV-ROLE-CF42-1 · 대조 ①) 복원 끝 확정 뒤 정착 창 닫힘 세부 — 사이클이 붙잡았던 대기열 항목(시작점 = quiescing
    /// 해제 **전**에 들어온 것)이 남아 있는 동안은 몰림 중이라 15초 조용함(대기열 막힘)에서만 닫는다(몰림 길이와 무관하게 몰림
    /// 전체가 최고치에 든다 · 몰림만으로 발화 없음). 다 배달되면 **마지막 배달 뒤에 시작한** ≥2초 조용함(그 배달 턴의 끝)에서
    /// 닫는다 — 배달 전부터 이어진 조용함(인계 직후 · 출력 전)은 닫지 않는다. 시작점 뒤에 들어온 항목(새 작업)은 몰림이 아니다.
    #[test]
    fn r2nc6_settle_window_closes_once_the_held_backlog_is_delivered() {
        let a = 110.0;
        let setup = |held: Option<f64>| {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
            clear(&mut g, &mut armed, 100.0);
            g.note_quiescing(a); // 붙여넣기 = 시작점
            g.note_queue(held);
            g.note_input(Some(a), a);
            assert_eq!(report(&mut g, &mut armed, 64, 60, W200K, a + 5.0), None);
            assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 30.0), None);
            g.note_delivery(Some(a + 33.0), None, a + 33.0); // 복원 턴 뒤 첫 대기열 배달 — 복원 끝 72 확정
            g.note_idle(a + 30.0, a + 33.0, a + 33.0);
            assert_eq!((g.restore_peak, g.restore_freeze_at), (Some(72), Some(a + 33.0)));
            assert_eq!(report(&mut g, &mut armed, 76, 60, W200K, a + 45.0), None);
            (g, armed)
        };
        // 붙잡혔던 항목이 남았다(clear 1초 뒤 enqueue) — 시작점 + 최소 창 뒤의 14초 조용함도 닫지 않는다 · 15초는 닫는다.
        let (mut g, _) = setup(Some(101.0));
        g.note_idle(a + 100.0, a + 114.0, a + 114.0);
        assert!(g.settling(a + 114.0), "붙잡혔던 배달이 남았는데 15초 미만 조용함이 창을 닫았다(몰림이 성장으로 셈 · ①)");
        g.note_idle(a + 100.0, a + 115.0, a + 115.0);
        assert!(!g.settling(a + 115.0), "대기열이 막힌 15초 조용함이 창을 닫지 않았다");
        // 다 배달됐다(마지막 배달 a+80) — 그 배달 턴 뒤 3초 조용함이 닫는다. 그 뒤 작업은 성장(막대 = 최고치 78 의 미확인 막대 80).
        let (mut g, mut armed) = setup(Some(101.0));
        g.note_delivery(Some(a + 80.0), None, a + 80.0);
        g.note_queue(None);
        g.note_idle(a + 78.0, a + 81.0, a + 81.0);
        assert!(g.settling(a + 81.0), "마지막 배달 전부터 이어진 조용함(인계 직후 · 턴 전)이 창을 닫았다");
        assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, a + 90.0), None); // 마지막 배달의 턴
        g.note_idle(a + 91.0, a + 94.0, a + 94.0);
        assert!(!g.settling(a + 94.0), "몰림이 끝난 뒤의 3초 조용함이 창을 닫지 않았다(바쁜 좌석 작업이 바닥에 든다 · ②)");
        assert_eq!(report(&mut g, &mut armed, 79, 60, W200K, a + 120.0), None);
        assert_eq!(g.settle_peak, Some(78));
        assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, a + 150.0), post_split(72, 78, CtxFloorRegime::Limited));
        // 남은 항목이 복원 끝(첫 배달 a+33 — 복원 끝 확정) **뒤**에 들어온 것(새 작업)이면 몰림이 아니다 — 3초 조용함이 닫는다.
        let (mut g, _) = setup(None);
        g.note_queue(Some(a + 40.0));
        g.note_idle(a + 57.0, a + 60.0, a + 60.0);
        assert!(!g.settling(a + 60.0), "복원 끝 뒤에 들어온 항목을 몰림으로 봤다");
        // ★(RV2NC-E20-1) 복원 턴 **도중**(시작점 뒤 · 복원 끝 전 a+20)에 대기열에 선 항목은 몰림이다 — 복원된 좌석이 부른 회신
        //   몰림(드릴 rv-d1p)이 그 모양이다. 종전(시작점 기준)은 몰림 한가운데서 닫아 나머지를 성장으로 셌다.
        let (mut g, _) = setup(Some(a + 20.0));
        g.note_idle(a + 57.0, a + 60.0, a + 60.0);
        assert!(g.settling(a + 60.0), "복원 턴 도중에 선 항목(회신 몰림)을 새 작업으로 봤다");
        g.note_idle(a + 60.0, a + 75.0, a + 75.0);
        assert!(!g.settling(a + 75.0), "몰림이 막힌 15초 조용함이 창을 닫지 않았다");
        // 1.9초 틈은 여전히 닫지 않는다 · 시작점 + 최소 창 전에도 닫지 않는다.
        let (mut g, _) = setup(None);
        g.note_idle(a + 57.1, a + 59.0, a + 59.0);
        g.note_idle(a + 50.0, a + 59.0, a + 59.0);
        assert!(g.settling(a + 59.0));
    }

    /// ★(R2NC6-1 · 뒷문) 복원 끝에 여유가 있는 세션이 차단 상한 **이하 막대**(정착 창 최고치 + 1 ≤ 차단 상한)에서 창 밖 발화하면
    /// 뒷문을 다시 무장한다(정착 창 최고치가 76~79 라 영역이 Probe 로 보여도 — 복원 끝이 차단기 재료가 아니다). 최고치가 차단 상한
    /// 이상(몰림이 여유보다 컸다 · 막대 = 최고치 + 1)인 세션은 다시 무장하지 않는다 — 몰림만으로 뒷문이 도는 교대 고리 없음(①).
    #[test]
    fn r2nc6_roomy_session_fire_at_the_hard_cap_rearms_the_backstop() {
        for (settled, rearm) in [(76u8, true), (79, true), (80, false), (82, false)] {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
            g.backstop_spent = true;
            clear(&mut g, &mut armed, 100.0);
            g.note_quiescing(110.0);
            let a = 110.0;
            g.note_input(Some(a), a);
            assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 30.0), None);
            g.note_delivery(Some(a + 33.0), None, a + 33.0);
            g.note_idle(a + 30.0, a + 33.0, a + 33.0);
            assert_eq!(report(&mut g, &mut armed, settled, 60, W200K, a + 45.0), None);
            let to = a + 46.0 + CTX_FLOOR_BURST_QUIET_SECS;
            g.note_idle(a + 46.0, to, to); // 몰림 뒤 15초 조용함 — 창 닫힘
            assert!(!g.settling(to));
            // ★(RV2NC-E20-1) 얇은 세션(최고치 ≥ 76)의 미확인 막대 = max(차단 상한, 최고치 + 2) — 발화가 실제 성장 ≥ 1%p 를 입증한다.
            let bar = settled.saturating_add(2).max(80);
            assert_eq!(g.effective_threshold(60, W200K, a + 899.0), bar, "최고치 {settled}");
            let f = report(&mut g, &mut armed, bar, 60, W200K, a + 900.0).expect("여유 세션의 창 밖 발화");
            assert_eq!((f.regime, f.floor, f.settle_backstop), (Some(CtxFloorRegime::Limited), Some(72), false));
            assert_eq!(!g.backstop_spent, rearm, "최고치 {settled}: 뒷문 재무장 {rearm} 이어야 한다");
        }
    }

    /// ★(R1-RUNAWAY-1 · ①) **압축 뒤 지침 재읽기 고리** — clear + 붙여넣기 + 복원이 이미 Claude 선제 압축점(83.5~90%) 위인 유휴
    /// 좌석(주기 신호만 · 분당 0.03%p)에서, 압축 뒤 SessionStart:compact 훅이 시킨 지침 재읽기로 압축 뒤 수준이 기본 임계 위
    /// (30 + 25~45%p = 55~75%)면 d9638152 는 재무장(기본 임계)이 그 수준에서 곧바로 발화 → 사이클의 붙여넣기가 다시 압축 →
    /// 재읽기 → 재무장 → 발화를 주기 신호마다 돌았다(4시간 19~21회). 재무장은 압축 뒤 바닥(재읽기 턴까지)을 다시 재고 그 위로
    /// 실제로 자란 뒤에만 발화한다 — 사이클 수는 실제 성장으로만 묶인다(압축 수로 셈하지 않는다 · R1-RUNAWAY-2).
    /// 사이클 = 앞 발화의 clear 가 난 뒤의 발화. 앞 발화의 clear **전에** Claude 가 먼저 압축하고 재읽기가 기본 임계를 다시
    /// 넘으면 그 교차가 한 번 더 난다(clear 뒤 세션이 아니다 = 기본 임계 · 종전 래치 거동 · 경보 라우터 쿨다운 5분이 접는다) —
    /// 그것은 새 사이클이 아니므로 따로 세고 1회 이하로 묶는다.
    #[test]
    fn r1_runaway_post_compaction_reread_does_not_refire_an_idle_seat() {
        let mut bad = vec![];
        for win in [W200K, None] {
            // 붙여넣기+복원만으로 선제 압축점 위(83.5~90) · 또는 참으로 가득 찬 CEO(77.8) + 사이클이 붙잡은 배달 6.5%p(3턴).
            for (paste, restore, backlog, turns) in [(76.0, 7.5, 0.0, 0u32), (76.0, 9.0, 0.0, 0), (78.0, 12.0, 0.0, 0), (69.35, 8.45, 6.5, 3)] {
                for reread in [0.0, 25.0, 30.0, 33.0, 40.0, 45.0] {
                    for rate in [0.0, 0.03] {
                        let seat = Seat { work_after: 20.0, work_per_min: rate, turn_secs: 5.0, reread, backlog, backlog_turns: turns,
                                          ..Seat::real(60, win, paste, restore) };
                        let run = run_seat(seat, 4.0 * 3600.0);
                        let grown = rate * 4.0 * 60.0;
                        let bound = 2 + (grown / f64::from(CTX_FLOOR_MIN_ROOM)).floor() as usize;
                        // 앞 발화의 clear(발화 + cycle_delay) 전의 재교차는 새 사이클이 아니다.
                        let refires = run.fires.windows(2).filter(|w| w[1].0 - w[0].0 < seat.cycle_delay).count();
                        let cycles = run.fires.len() - refires;
                        let fires: Vec<_> = run.fires.iter().map(|(t, p, f, _)| (*t as u32, *p, f.regime, f.floor, f.after_compaction)).collect();
                        let line = format!("{win:?} 붙여넣기 {paste}+{restore} 몰림 {backlog} 재읽기 +{reread} 분당 {rate}: 사이클 {cycles} 재교차 {refires} 압축 {} \
                                            재무장 {} {fires:?}", run.compactions.len(), run.rearms);
                        if cycles > bound || refires > 1 {
                            bad.push(line);
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "(①) 유휴 좌석이 압축 → 재읽기 → 재무장 발화 고리를 돈다 — {} 건:\n{}", bad.len(), bad.join("\n"));
    }

    /// ★(R1-RUNAWAY-1 · ① · ②) 일하는 좌석에서도 압축 뒤 발화는 **압축 뒤 잰 바닥 위로 실제로 자란 뒤**다 — 참으로 가득 찬
    /// 200K·창 미상 CEO(복원 끝 77.8) · 배달 몰림 6.5%p(3턴) · 압축 뒤 재읽기 0·33%p · 분당 0.2%p(5분에 1%p) · 8시간:
    /// 압축 뒤 발화의 pct ≥ 압축 뒤 잰 바닥 + 최소 여유(Probe 1%p) · 기본 임계 이상 · ★(ADV2-R1-1) clear 로 돌아오는 바닥 + 1 이상 ·
    /// 사이클 수 ≤ 2 + 성장 ÷ 최소 여유(압축 수를 셈하지 않는다). Claude 압축은 여전히 재무장한다 — clear 바닥이 선제 압축점 아래면
    /// 압축 뒤 cys 사이클이 난다(R2NC5-1 (b) 유지) · 위(몰림 6.5 → 84)면 없다(그 clear 가 곧바로 다음 압축을 부른다).
    #[test]
    fn r1_runaway_after_compaction_fire_needs_growth_above_the_post_compaction_floor() {
        for win in [W200K, None] {
            for reread in [0.0, 33.0] {
                for (backlog, turns) in [(0.0, 0u32), (6.5, 3)] {
                    let seat = Seat { restore_secs: 30.0, backlog, backlog_turns: turns, work_after: 20.0, work_per_min: 0.2, turn_secs: 20.0,
                                      reread, ..Seat::real(60, win, 69.35, 8.45) };
                    let run = run_seat(seat, 8.0 * 3600.0);
                    let ctx = format!("{win:?} 재읽기 +{reread} 몰림 {backlog}: 발화 {:?} 압축 {:?}", run.fires, run.compactions);
                    let grown = 0.2 * 8.0 * 60.0;
                    assert!((run.fires.len() as f64) <= 2.0 + grown / f64::from(CTX_FLOOR_MIN_ROOM), "(①) 사이클이 성장보다 많다 — {ctx}");
                    assert!(run.rearms <= run.compactions.len(), "{ctx}");
                    let after: Vec<_> = run.fires.iter().filter(|f| f.2.after_compaction).collect();
                    let landing = (69.35 + 8.45 + backlog).round() as u8;
                    for (_, p, f, _) in &after {
                        let floor = f.floor.expect("압축 뒤 발화에는 압축 뒤 잰 바닥이 있다");
                        let min_room = if f.regime == Some(CtxFloorRegime::Probe) { 1 } else { CTX_FLOOR_MIN_ROOM };
                        assert!(*p >= 60 && p.saturating_sub(floor) >= min_room, "(①) 압축 뒤 바닥 {floor} 위 {p} 에서 발화 — {ctx}");
                        assert!(*p > landing, "(ADV2-R1-1) 압축 뒤 발화 {p} 가 clear 로 돌아오는 바닥 {landing} 이하 — {ctx}");
                    }
                    if !run.compactions.is_empty() && reread == 0.0 && landing + 1 < 83 {
                        assert!(!after.is_empty(), "(②) Stopped 좌석이 압축 뒤 cys 사이클을 다시 받지 못한다 — {ctx}");
                    }
                    if landing + 1 >= 84 {
                        assert!(after.is_empty(), "(ADV2-R1-1) clear 바닥이 선제 압축점 위인 좌석에 압축 뒤 cys 사이클 — {ctx}");
                    }
                }
            }
        }
    }

    /// ★(R2NC6-1 · RV-ROLE-CF42-1 · ① · ② 격자) 몰림 판정을 **대기열 사실**(시작점 전에 들어온 항목이 남았나)로 하면 몰림 길이와
    /// 작업 틈에 무관하게 두 불변식이 함께 선다:
    /// (①) 유휴 좌석(참 바닥 72~74)이 사이클마다 붙잡힌 배달 5~12%p 를 1~8턴 × 5~30초(몰림 5~264초)로 받아도 재발화 없음 —
    ///     부트 1회 + 몰림이 차단 상한을 넘으면 뒷문 1회. (시간 수명 60초로 자르면 몰림이 60초를 넘는 160/1152 경우가 2시간에
    ///     19~35회 도는 고리였다 — 기각한 설계.)
    /// (②) 바쁜 여유 좌석(참 바닥 56~74 · 붙잡힌 배달 0~7%p · 분당 0.5~3%p · 턴 20~57초)은 Claude 선제 압축 0 · clear 는 82 이하
    ///     (분당 3%p 에서 보고 간격만큼 넘어 보임). d9638152 는 같은 격자에서 282/660 이 83% 이상·압축이었다.
    #[test]
    fn r2nc6_held_backlog_of_any_length_is_absorbed_and_busy_seats_clear_before_compaction() {
        let mut bad = vec![];
        for win in [W200K, None] {
            for (paste, restore) in [(63.3, 8.9), (72.0, 0.0), (70.0, 4.0)] {
                for bl in [5.0, 8.0, 9.0, 12.0] {
                    for bt in [1u32, 2, 3, 4, 6, 8] {
                        for bts in [5.0, 10.0, 20.0, 30.0] {
                            for (rs, qg) in [(14.0, 0.3), (53.0, 3.0)] {
                                let seat = Seat { restore_secs: rs, backlog: bl, backlog_turns: bt, backlog_turn_secs: bts, queue_gap: qg,
                                                  ..Seat::real(60, win, paste, restore) };
                                let run = run_seat(seat, 2.0 * 3600.0);
                                // (몰림만으로 선제 압축점 위(72 + 12)면 Claude 가 압축한다 — 그것은 고리가 아니다 · 발화 수만 본다.)
                                if run.fires.len() > 2 {
                                    bad.push(format!("(①) 유휴 {win:?} {paste}+{restore} 몰림 {bl}%p/{bt}×{bts}s: 발화 {:?}", run.fires));
                                }
                            }
                        }
                    }
                }
            }
            for (paste, restore) in [(63.3, 8.9), (70.0, 4.0), (51.0, 5.0)] {
                for rate in [0.5, 0.8, 1.0, 1.3, 2.0, 3.0] {
                    for turn in [20.0, 40.0, 46.0, 52.0, 57.0] {
                        for (bl, bt) in [(0.0, 0u32), (3.3, 2), (5.0, 3), (7.0, 3)] {
                            if paste + restore + bl >= 80.0 {
                                continue; // 붙잡힌 배달만으로 차단 상한 위 — 여유 좌석이 아니다
                            }
                            let seat = Seat { restore_secs: 30.0, work_after: 5.0, work_per_min: rate, turn_secs: turn, backlog: bl,
                                              backlog_turns: bt, ..Seat::real(60, win, paste, restore) };
                            let run = run_seat(seat, 3.0 * 3600.0);
                            let late = run.fires.iter().skip(1).any(|(_, p, f, _)| f.after_compaction || *p > 82);
                            if run.fires.len() < 3 || !run.compactions.is_empty() || late {
                                bad.push(format!("(②) 바쁜 {win:?} {paste}+{restore} 몰림 {bl}/{bt} 분당 {rate} 턴 {turn}s: 발화 {:?} 압축 {:?}",
                                                 run.fires.iter().map(|f| (f.0 as u32, f.1)).collect::<Vec<_>>(), run.compactions));
                            }
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{} 건:\n{}", bad.len(), bad.join("\n"));
    }

    // ───────────── ★(RV2NC-E20-1 · RV2NC-E20-2 · RV2-ROLE-1 · ADV2-R1-1) 재검증 3회차 반례 ─────────────

    /// 사이클 수(부트 교차 포함) — 앞 발화의 clear(발화 + cycle_delay) **전의** 재교차는 새 사이클이 아니다(경보 라우터가 접는다).
    fn cycles(run: &Run, seat: &Seat) -> usize {
        let refires = run.fires.windows(2).filter(|w| w[1].0 - w[0].0 < seat.cycle_delay).count();
        run.fires.len() - refires
    }

    /// ★(RV2NC-E20-1 · ① 회귀 · 재검증자 모형 그대로) **사이클마다 몰림이 정착 최고치를 다시 채우는 유휴 좌석**은 주기 신호만으로
    /// 매 세션 다시 발화하지 않는다 — 작성자 유휴 검체의 좌석(참 바닥 72~74 · 사이클이 붙잡은 몰림 5~9%p)과 재검증자 격자(붙잡힌
    /// 몰림 · **복원 끝 2초 뒤 한꺼번에 선 회신 몰림**(드릴 rv-d1p))에 주기 신호(분당 0.03·0.1·0.15%p · 5초 턴)를 더해 4시간:
    /// 사이클 수 ≤ 2 + 성장/5(성장 = 주기 신호 몫만 — 몰림은 사이클이 부른 것이다). e20f1cb6 은 여유 있는 복원 끝이 확인을 영구히
    /// 꺼 막대가 최고치 + 1 에 머물렀다(4시간 14~52회 · 드릴 1시간 10~11회). 얇은 세션은 1회 뒤 몰림을 바닥으로 확정한다(Backlog).
    #[test]
    fn rv2nc_e20_idle_seat_with_a_recurring_backlog_does_not_loop_on_heartbeats() {
        let mut bad = vec![];
        let mut backlog_seen = 0;
        for win in [W200K, None] {
            for (paste, restore, backlog, turns) in [(63.3, 8.9, 5.0, 3u32), (72.0, 0.0, 5.0, 1), (70.0, 4.0, 7.0, 3), (72.0, 0.0, 9.0, 3),
                                                     (63.3, 8.9, 8.0, 3), (70.0, 4.0, 5.0, 3), (68.0, 4.0, 6.0, 2)] {
                for reply in [None, Some(2.0)] {
                    for rate in [0.0, 0.03, 0.1, 0.15] {
                        for (restore_secs, queue_gap) in [(30.0, 0.3), (14.0, QUEUE_QUIET)] {
                            let seat = Seat { restore_secs, backlog, backlog_turns: turns, backlog_reply: reply, queue_gap, work_after: 20.0,
                                              work_per_min: rate, turn_secs: 5.0, ..Seat::real(60, win, paste, restore) };
                            let run = run_seat(seat, 4.0 * 3600.0);
                            let bound = 2 + (rate * 240.0 / f64::from(CTX_FLOOR_MIN_ROOM)).floor() as usize;
                            let n = cycles(&run, &seat);
                            backlog_seen += run.fires.iter().filter(|f| f.2.regime == Some(CtxFloorRegime::Backlog)).count();
                            if n > bound {
                                bad.push(format!("{win:?} {paste}+{restore} 몰림 {backlog}/{turns} 회신 {reply:?} 분당 {rate} 복원 {restore_secs}s: \
                                                  사이클 {n} > {bound} {:?}",
                                                 run.fires.iter().map(|f| (f.0 as u32, f.1, f.2.regime, f.2.settled)).collect::<Vec<_>>()));
                            }
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "(①) 몰림이 되풀이되는 유휴 좌석이 주기 신호만으로 매 세션 발화한다 — {} 건:\n{}", bad.len(), bad.join("\n"));
        assert!(backlog_seen > 0, "이 검체가 Backlog(몰림 확정) 발화를 한 번도 내지 않았다 — 반례 모양을 재현하지 못한다");
    }

    /// ★(RV2NC-E20-1 · ② 대조 · 빈도 상한의 다른 쪽) 같은 몰림이 되풀이되는 좌석이라도 **일하면**(분당 0.5~3%p) 얇은 세션의 발화가
    /// 빠른 성장이라 확인되지 않는다 — clear 는 차단 상한 근처(≤ 82)에서 · Claude 선제 압축 0 · Backlog 없음(R2NC6 바쁜 격자와 같은
    /// 좌석 · 회신 몰림 포함). 종전 fba29db0 은 이 좌석을 Probe → Stopped(86~88)로 보내 매 세션 Claude 압축이 먼저 왔다.
    #[test]
    fn rv2nc_e20_busy_seat_with_a_recurring_backlog_keeps_clearing_near_the_hard_cap() {
        let mut bad = vec![];
        for win in [W200K, None] {
            for (paste, restore) in [(63.3, 8.9), (70.0, 4.0)] {
                for (bl, bt) in [(3.3, 2u32), (5.0, 3), (7.0, 3)] {
                    if paste + restore + bl >= 80.0 {
                        continue;
                    }
                    for reply in [None, Some(2.0)] {
                        for rate in [0.5, 1.0, 2.0, 3.0] {
                            for turn in [20.0, 46.0, 57.0] {
                                let seat = Seat { restore_secs: 30.0, work_after: 5.0, work_per_min: rate, turn_secs: turn, backlog: bl,
                                                  backlog_turns: bt, backlog_reply: reply, ..Seat::real(60, win, paste, restore) };
                                let run = run_seat(seat, 3.0 * 3600.0);
                                let late = run.fires.iter().skip(1).any(|(_, p, f, _)| {
                                    f.after_compaction || *p > 82 || matches!(f.regime, Some(CtxFloorRegime::Backlog | CtxFloorRegime::Probe | CtxFloorRegime::Stopped))
                                });
                                if run.fires.len() < 3 || !run.compactions.is_empty() || late {
                                    bad.push(format!("{win:?} {paste}+{restore} 몰림 {bl}/{bt} 회신 {reply:?} 분당 {rate} 턴 {turn}s: 발화 {:?} 압축 {:?}",
                                                     run.fires.iter().map(|f| (f.0 as u32, f.1, f.2.regime)).collect::<Vec<_>>(), run.compactions));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "(②) 몰림이 되풀이되는 바쁜 좌석의 cys clear 가 늦었다 — {} 건:\n{}", bad.len(), bad.join("\n"));
    }

    /// ★(RV2NC-E20-1 · 세부) 얇은 세션의 발화 판정 — 미확인 막대는 최고치 + 2(79 → 81)이고, 정착 창이 닫힌 뒤 느린 성장(주기
    /// 신호)으로 닿으면 다음 세션이 몰림을 바닥으로 확정(Backlog · 최고치 + 6) · 빠른 성장(일하는 좌석)이면 확인하지 않는다. Backlog
    /// 세션의 빠른 발화도 확인을 푼다. 복원 끝이 차단기 높이인 세션(참으로 가득 참)은 종전대로 Probe → Stopped.
    #[test]
    fn rv2nc_e20_thin_session_fire_is_classified_by_growth_speed() {
        // clear → 붙여넣기(시작점 a) → 복원 72 → 대기열 배달(복원 끝 확정) → 몰림 턴 → 15초 조용함(창 닫힘) 뒤 settled.
        let session = |g: &mut CtxLoopGuard, armed: &mut bool, t0: f64, restore: u8, settled: u8| -> f64 {
            clear(g, armed, t0);
            let a = t0 + 10.0;
            g.note_quiescing(a);
            g.note_input(Some(a), a);
            assert_eq!(report(g, armed, restore, 60, W200K, a + 30.0), None);
            g.note_delivery(Some(a + 33.0), None, a + 33.0);
            g.note_idle(a + 30.0, a + 33.0, a + 33.0);
            assert_eq!(report(g, armed, settled, 60, W200K, a + 45.0), None);
            g.note_idle(a + 46.0, a + 90.0, a + 90.0);
            assert!(!g.settling(a + 90.0), "창이 닫히지 않았다");
            a + 90.0
        };
        // 느린 얇은 발화(79 → 81 을 40분에) → 다음 세션 Backlog 85(79 + 6) → 빠른 Backlog 발화(85 를 3분에) → 다음 세션 Limited 81.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        let close = session(&mut g, &mut armed, 100.0, 72, 79);
        assert_eq!(g.effective_threshold(60, W200K, close), 81, "얇은 미확인 막대 = 최고치 + 2");
        assert_eq!(report(&mut g, &mut armed, 80, 60, W200K, close + 1_200.0), None);
        assert_eq!(report(&mut g, &mut armed, 81, 60, W200K, close + 2_400.0), post_split(72, 79, CtxFloorRegime::Limited));
        assert!(g.session_tier3 && !g.session_fast, "느린 얇은 발화가 확인 재료가 아니다");
        let close = session(&mut g, &mut armed, 3_000.0, 72, 79);
        assert_eq!(g.effective_threshold(60, W200K, close), 85, "몰림을 바닥으로 확정(Backlog 79 + 6)하지 않았다");
        assert_eq!(report(&mut g, &mut armed, 85, 60, W200K, close + 180.0), post_split(72, 79, CtxFloorRegime::Backlog));
        assert!(g.session_fast, "3분에 6%p — 일하는 좌석");
        let close = session(&mut g, &mut armed, 4_000.0, 72, 79);
        assert_eq!(g.effective_threshold(60, W200K, close), 81, "빠른 발화가 확인을 풀지 않았다");
        // 빠른 얇은 발화(77 → 80 을 4분에 — 분당 0.5%p)는 확인 재료가 아니다 · 경계(표시 성장 − 1 = 2%p 를 10분 = 분당 0.2%p)는 빠르다.
        for (dt, fast) in [(240.0, true), (600.0, true), (601.0, false), (1_800.0, false)] {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
            let close = session(&mut g, &mut armed, 100.0, 72, 77);
            assert_eq!(g.effective_threshold(60, W200K, close), 80);
            assert!(report(&mut g, &mut armed, 80, 60, W200K, close + dt).is_some());
            assert_eq!((g.session_fast, g.session_tier3), (fast, !fast), "3%p 를 {dt}초 — 빠름 {fast}");
        }
        // 참으로 가득 찬 좌석(복원 끝 78) — 종전 그대로 Probe → Stopped(성장 속도와 무관).
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        let close = session(&mut g, &mut armed, 100.0, 78, 79);
        assert_eq!(g.bar(60, W200K).map(|b| (b.bar, b.regime, b.thin)), Some((80, CtxFloorRegime::Probe, false)));
        assert!(report(&mut g, &mut armed, 80, 60, W200K, close + 60.0).is_some());
        let close = session(&mut g, &mut armed, 1_000.0, 78, 79);
        assert_eq!(g.bar(60, W200K).map(|b| (b.bar, b.regime)), Some((89, CtxFloorRegime::Stopped)));
        let _ = close;
    }

    /// ★(RV2NC-E20-2 · ②) 복원 턴 **도중** 바쁨을 보지 않는 기계 주입(채널 행 — quiescing 해제 직후 보류분 · writer Inject)은 Claude 가
    /// 복원 턴 끝에 조용함 없이 곧바로 이어 처리한다 — 그 턴을 복원 끝에 실어 여유 있는 master(참 바닥 72)를 Probe → Stopped 로
    /// 오판하지 않는다: 복원 턴 오염으로 적고 그 입력 때의 최고치(하한)로 차단기를 판정한다. 운영 신호 모형 · 200K·창 미상 · 채널
    /// 3~6%p(주입: 붙여넣기 2~25초 뒤) · 일(분당 0.5·1%p) · 3시간: clear 뒤 발화는 Raise·Limited · 81 이하 · Claude 압축 0 · 자동
    /// 압축을 끈 좌석도 차단점(88.5%) 근처로 끌려가지 않는다. 유휴(주기 신호만)면 고리 없음(사이클 ≤ 2 + 성장/5).
    /// 종전 e20f1cb6 은 드릴 rv-d3(19.7KB 채널 행)에서 floor 76 probe → stopped 86 → Claude 압축 · 자동 압축 끔 rv-d3n 은 막대 89.
    #[test]
    fn rv2nc_e20_channel_row_during_the_restore_turn_does_not_trip_the_breaker() {
        let mut bad = vec![];
        for win in [W200K, None] {
            for (paste, restore) in [(63.3, 8.9), (72.0, 0.0)] {
                for (off, pp) in [(2.0, 4.3), (10.0, 3.0), (25.0, 6.0)] {
                    for autocompact in [true, false] {
                        for rate in [0.5, 1.0] {
                            let base = Seat { restore_secs: 30.0, chan: Some((off, pp, 12.0)), work_after: 20.0, work_per_min: rate,
                                              turn_secs: 20.0, ..Seat::real(60, win, paste, restore) };
                            let seat = if autocompact { base } else { Seat { compact_at: None, ..base } };
                            let run = run_seat(seat, 3.0 * 3600.0);
                            let wrong = run.fires.iter().skip(1).any(|(_, p, f, _)| {
                                *p > 81 || !matches!(f.regime, Some(CtxFloorRegime::Raise | CtxFloorRegime::Limited))
                            });
                            if run.fires.len() < 3 || !run.compactions.is_empty() || wrong || run.max_post_clear >= 84 {
                                bad.push(format!("{win:?} {paste}+{restore} 채널 +{off}s {pp}%p 자동압축 {autocompact} 분당 {rate}: 발화 {:?} 압축 {:?} 최고 {}",
                                                 run.fires.iter().map(|f| (f.0 as u32, f.1, f.2.regime, f.2.floor, f.2.settled)).collect::<Vec<_>>(),
                                                 run.compactions, run.max_post_clear));
                            }
                        }
                        // 유휴(주기 신호 분당 0.03·0.1%p) — 채널 행이 사이클마다 와도 고리 없음.
                        for rate in [0.03, 0.1] {
                            let base = Seat { restore_secs: 30.0, chan: Some((off, pp, 12.0)), work_after: 20.0, work_per_min: rate,
                                              turn_secs: 5.0, ..Seat::real(60, win, paste, restore) };
                            let seat = if autocompact { base } else { Seat { compact_at: None, ..base } };
                            let run = run_seat(seat, 4.0 * 3600.0);
                            let bound = 2 + (rate * 240.0 / f64::from(CTX_FLOOR_MIN_ROOM)).floor() as usize;
                            if cycles(&run, &seat) > bound {
                                bad.push(format!("(①) 유휴 {win:?} {paste}+{restore} 채널 +{off}s {pp}%p 분당 {rate}: 사이클 {} > {bound}",
                                                 cycles(&run, &seat)));
                            }
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{} 건:\n{}", bad.len(), bad.join("\n"));
        // 순수: 붙여넣기 뒤 채널 Inject(대기열 배달 아님) → 오염 · 하한 확정 · 그 입력의 턴(76)은 차단기 재료가 아니다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        g.note_quiescing(110.0);
        let a = 110.0;
        g.note_input(Some(a + 0.5), a + 0.5); // 붙여넣기(직접 send)
        assert_eq!(g.paste_input_at, Some(a + 0.5));
        assert_eq!(report(&mut g, &mut armed, 67, 60, W200K, a + 15.0), None);
        g.note_delivery(None, Some(a + 16.0), a + 16.0); // 채널 행 Inject 끝 — 복원 턴 도중
        assert!(g.restore_tainted && g.restore_peak == Some(67), "복원 턴 도중 채널 Inject 를 오염으로 적지 않았다");
        assert_eq!(report(&mut g, &mut armed, 72, 60, W200K, a + 30.0), None);
        assert_eq!(report(&mut g, &mut armed, 76, 60, W200K, a + 36.0), None); // 채널 턴(조용함 없이 이어짐)
        g.note_idle(a + 40.0, a + 80.0, a + 80.0);
        let b = g.bar(60, W200K).unwrap();
        assert_eq!((b.regime, b.floor, b.settled, b.tainted, b.bar), (CtxFloorRegime::Limited, 76, 76, true, 80), "{b:?}");
        // 대기열 배달의 CR(인계 뒤 5초 안)은 오염 재료가 아니다 · 붙여넣기 자신(첫 입력)도 아니다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        g.note_quiescing(110.0);
        g.note_input(Some(a + 0.5), a + 0.5);
        g.note_delivery(None, Some(a + 0.5), a + 1.0); // 붙여넣기 자신이 Inject 로 기록된 모형(검체·재검증자 모형)
        assert!(!g.restore_tainted, "붙여넣기 자신을 오염으로 봤다");
        g.note_delivery(Some(a + 2.0), Some(a + 2.4), a + 2.4); // 상승 보고 전 대기열 배달(우회한 붙여넣기)
        assert!(!g.restore_tainted, "대기열 배달의 CR 을 오염으로 봤다");
        // 참으로 가득 찬 좌석(붙여넣기 턴이 이미 77 을 보고한 뒤 채널 행) — 하한 77 이 차단기 높이라 종전대로 Probe.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        g.note_quiescing(110.0);
        g.note_input(Some(a + 0.5), a + 0.5);
        assert_eq!(report(&mut g, &mut armed, 77, 60, W200K, a + 20.0), None);
        g.note_delivery(None, Some(a + 21.0), a + 21.0);
        assert_eq!(report(&mut g, &mut armed, 79, 60, W200K, a + 40.0), None);
        g.note_idle(a + 45.0, a + 80.0, a + 80.0);
        assert_eq!(g.bar(60, W200K).map(|b| (b.regime, b.thin, b.tainted)), Some((CtxFloorRegime::Probe, false, true)));
    }

    /// ★(RV2NC-E20-2) 복원 턴 보류 판정(`restore_turn_open`) — clear 뒤 세션 · 시작점 뒤 · 복원 끝 전(≥2초 조용함 없음) · 상한 120초
    /// 안에서만 참. 발화 이력 없음(부트)·압축 뒤 재는 창·복원 끝 확정 뒤·조용함 뒤·상한 뒤는 거짓(보류 없음).
    #[test]
    fn rv2nc_e20_restore_turn_hold_window() {
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert!(!g.restore_turn_open(10.0), "발화 이력 없는 좌석");
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        clear(&mut g, &mut armed, 100.0);
        g.note_quiescing(110.0);
        assert!(g.restore_turn_open(111.0) && g.restore_turn_open(110.0 + CTX_FLOOR_RESTORE_HOLD_SECS));
        assert!(!g.restore_turn_open(110.0 + CTX_FLOOR_RESTORE_HOLD_SECS + 0.5), "상한 뒤에도 보류");
        assert_eq!(report(&mut g, &mut armed, 64, 60, W200K, 120.0), None);
        g.note_idle(130.0, 133.0, 133.0); // 복원 턴 끝 후보
        assert!(!g.restore_turn_open(134.0), "복원 턴 끝 뒤에도 보류");
        // 압축 뒤 재는 창에는 복원 턴이 없다.
        let (mut g, mut armed) = (CtxLoopGuard::default(), true);
        assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
        for t0 in [100.0, 1_000.0] {
            clear(&mut g, &mut armed, t0);
            assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, t0 + 10.0), None);
            if t0 < 500.0 {
                assert!(report(&mut g, &mut armed, 80, 60, W200K, t0 + 700.0).is_some());
            }
        }
        let _ = report(&mut g, &mut armed, 86, 60, W200K, 2_000.0);
        assert!(g.observe(30, W200K, 2_100.0).is_some());
        assert!(!g.restore_turn_open(2_101.0), "압축 뒤 재는 창에서 보류");
    }

    /// ★(RV2-ROLE-1 · ② · 재검증자 rv2_e4·rv2_h3 모양) Claude 압축이 **긴 작업 턴 도중**(입력 없이 이어지는 도구 연쇄 · 턴 안 분당
    /// 1~3%p)에 와도 압축 뒤 바닥은 재읽기 몫(압축 + [`CTX_FLOOR_REREAD_SECS`])만 잰다 — 다시 잰 바닥 ≤ 압축 뒤 수준(30 + 재읽기) +
    /// 분당 성장 × 1.5분 + 2. 종전(e20f1cb6)은 정착 창(최소 60초 · 최대 600초 · 조용함 대기)이 턴 끝까지 열려 참 압축 뒤 수준 63~65 를
    /// 80 으로 재 Stopped · 압축 2~3회에 사이클 1회(또는 0회 — 6시간 압축만 26~52회)였다. clear 로 돌아오는 바닥(복원 끝 78)이
    /// 선제 압축점 아래인 좌석은 압축마다 cys 사이클을 되찾는다(압축 수 ≤ 사이클 수 + 1). 몰림 6.5%p 로 clear 바닥이 선제
    /// 압축점 위(84)인 좌석은 압축 뒤 cys 사이클이 없다(ADV2-R1-1 — 그 clear 가 곧바로 다음 압축을 부른다).
    #[test]
    fn rv2_role_mid_turn_compaction_remeasures_only_the_reread() {
        let mut bad = vec![];
        for win in [W200K, None] {
            for reread in [0.0, 33.0] {
                for rate in [1.0, 1.5, 2.0, 3.0] {
                    for (backlog, turns) in [(0.0, 0u32), (6.5, 3)] {
                        for inputs in [false, true] {
                            let seat = Seat { restore_secs: 30.0, backlog, backlog_turns: turns, work_after: 20.0, work_per_min: rate,
                                              turn_secs: 60.0, reread, work_inputs: inputs, compact_midturn: true,
                                              ..Seat::real(60, win, 69.35, 8.45) };
                            let run = run_seat(seat, 6.0 * 3600.0);
                            let ctx = format!("{win:?} 재읽기 +{reread} 분당 {rate} 몰림 {backlog} 입력 {inputs}: 발화 {:?} 압축 {:?}",
                                              run.fires.iter().map(|f| (f.0 as u32, f.1, f.2.regime, f.2.floor, f.2.after_compaction)).collect::<Vec<_>>(),
                                              run.compactions.iter().map(|c| *c as u32).collect::<Vec<_>>());
                            let level = 30.0 + reread;
                            let landing = (69.35 + 8.45 + backlog).round() as u8;
                            for (_, p, f, _) in run.fires.iter().filter(|f| f.2.after_compaction) {
                                let floor = f64::from(f.floor.expect("압축 뒤 잰 바닥"));
                                if floor > level + rate * 1.5 + 2.0 {
                                    bad.push(format!("다시 잰 바닥 {floor} > 재읽기 수준 {level} + 몇 %p — {ctx}"));
                                }
                                if *p <= landing {
                                    bad.push(format!("(ADV2-R1-1) 압축 뒤 발화 {p} ≤ clear 바닥 {landing} — {ctx}"));
                                }
                            }
                            let after = run.fires.iter().filter(|f| f.2.after_compaction).count();
                            if landing + 1 < 83 {
                                if run.compactions.len() > after + 1 {
                                    bad.push(format!("(②) 압축 {} 회에 압축 뒤 cys 사이클 {after} — {ctx}", run.compactions.len()));
                                }
                            } else if after > 0 {
                                bad.push(format!("(ADV2-R1-1) clear 바닥 {landing} 좌석의 압축 뒤 cys 사이클 {after} — {ctx}"));
                            }
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{} 건:\n{}", bad.len(), bad.join("\n"));
        // ★(RV2-ROLE-1 (b)) 압축 전에 들어와 남은 평상 대기열 항목(사이클 무관 회신 적체)이 압축 뒤 재는 창을 붙잡지 않는다 —
        //   창은 재읽기 몫([`CTX_FLOOR_REREAD_SECS`])에서 끝난다(재검증자 rv2_e5: 종전 14초 조용함에도 닫히지 않음).
        for aged in [None, Some(30.0)] {
            let (mut g, mut armed) = (CtxLoopGuard::default(), true);
            assert_eq!(report(&mut g, &mut armed, 70, 60, W200K, 0.0), FIRE);
            for t0 in [100.0, 1_000.0] {
                clear(&mut g, &mut armed, t0);
                g.note_quiescing(t0 + 10.0);
                assert_eq!(report(&mut g, &mut armed, 78, 60, W200K, t0 + 40.0), None);
                g.note_idle(t0 + 45.0, t0 + 400.0, t0 + 400.0);
                let _ = report(&mut g, &mut armed, 80, 60, W200K, t0 + 500.0);
            }
            assert_eq!(g.bar(60, W200K).map(|b| b.regime), Some(CtxFloorRegime::Stopped));
            let _ = report(&mut g, &mut armed, 86, 60, W200K, 2_500.0);
            let tc = 2_600.0;
            assert!(g.observe(30, W200K, tc).is_some());
            g.note_queue(aged.map(|a| tc - a));
            let _ = report(&mut g, &mut armed, 63, 60, W200K, tc + 20.0);
            g.note_delivery(Some(tc + 25.0), None, tc + 25.0);
            g.note_idle(tc + 21.0, tc + 25.0, tc + 25.0);
            assert_eq!(g.restore_peak, None, "압축 뒤 재는 창에서 복원 끝을 확정했다");
            assert!(!g.settling(tc + CTX_FLOOR_REREAD_SECS + 1.0), "대기열 {aged:?}: 압축 뒤 재는 창이 재읽기 몫에서 끝나지 않았다");
            assert_eq!(g.settle_peak, Some(63));
        }
    }

    /// ★(ADV2-R1-1 · ① · 재검증자 장기 반례) **참으로 가득 찬 좌석**(clear 바닥 76~85 — CEO 77.8 · 재검증자 cfg 76+9 · 몰림 포함)이
    /// 느리게 자라면(10분에 0.3%p ~ 분당 1%p) 24시간 동안: 압축 뒤 cys 발화는 언제나 clear 로 돌아오는 바닥 + 1 이상이다(그 clear 가
    /// 컨텍스트를 올리지 않는다) · clear 바닥이 선제 압축점 높이면 압축 뒤 cys 사이클이 없다 · 사이클 수 ≤ 2 + 성장/5 · 재무장 ≤ 압축.
    /// e20f1cb6 은 압축 뒤 바닥(63~70) 위 75·80 에서 발화해 그 clear 가 85 로 올라 곧바로 압축 → 재무장 → 발화를 성장 약 4.5%p 마다
    /// 돌았다(24시간 발화 11 · 압축 10 · 분당 1%p 좌석 4시간 발화 22).
    #[test]
    fn adv2_r1_truly_full_seat_is_not_cleared_below_its_clear_floor_after_compaction() {
        let mut bad = vec![];
        for win in [W200K, None] {
            for (paste, restore, backlog, turns) in [(76.0, 9.0, 0.0, 0u32), (69.35, 8.45, 0.0, 0), (69.35, 8.45, 3.3, 2), (70.0, 7.0, 0.0, 0)] {
                for reread in [33.0, 40.0, 45.0] {
                    for (rate, turn) in [(0.03, 5.0), (0.2, 20.0), (1.0, 20.0)] {
                        let seat = Seat { restore_secs: 30.0, backlog, backlog_turns: turns, work_after: 20.0, work_per_min: rate, turn_secs: turn,
                                          reread, compact_midturn: true, ..Seat::real(60, win, paste, restore) };
                        let hours = if rate < 0.1 { 24.0 } else { 8.0 };
                        let run = run_seat(seat, hours * 3600.0);
                        let landing = (paste + restore + backlog).round() as u8;
                        let ctx = format!("{win:?} {paste}+{restore} 몰림 {backlog} 재읽기 +{reread} 분당 {rate}: 발화 {:?} 압축 {}",
                                          run.fires.iter().map(|f| (f.0 as u32, f.1, f.2.regime, f.2.after_compaction)).collect::<Vec<_>>(),
                                          run.compactions.len());
                        let bound = 2 + (rate * hours * 60.0 / f64::from(CTX_FLOOR_MIN_ROOM)).floor() as usize;
                        if cycles(&run, &seat) > bound {
                            bad.push(format!("(①) 사이클 {} > {bound} — {ctx}", cycles(&run, &seat)));
                        }
                        if run.rearms > run.compactions.len() {
                            bad.push(format!("재무장 {} > 압축 — {ctx}", run.rearms));
                        }
                        for (_, p, f, _) in run.fires.iter().filter(|f| f.2.after_compaction) {
                            if *p <= landing {
                                bad.push(format!("(①) 압축 뒤 발화 {p} ≤ clear 바닥 {landing} — {ctx} {f:?}"));
                            }
                        }
                        if landing >= 83 && run.fires.iter().any(|f| f.2.after_compaction) {
                            bad.push(format!("(①) clear 바닥 {landing}(선제 압축점) 좌석의 압축 뒤 cys 사이클 — {ctx}"));
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{} 건:\n{}", bad.len(), bad.join("\n"));
    }
}
