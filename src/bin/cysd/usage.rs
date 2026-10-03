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
    /// ★0.14.43(B3 · 오너 결재 "경보 입력 = 리셋 전 ∧ (사용 중 ∨ 관측 나이 ≤ 1800초)"): 이 `rate` 가 **새로 생산된** 시각(epoch 초 ·
    /// 0.0 = 모름). 상태줄 보고(`usage.report`)·codex rollout 의 신선 rate = 그 순간이고, claude transcript tail 처럼 `prev.rate` 를
    /// **이월**한 스냅샷은 이 값도 이월한다 — `updated_at` 은 이월에도 `now` 로 찍히므로 rate 의 나이가 아니다(이월이 유휴 좌석의
    /// 옛 값을 신선한 것으로 둔갑시키던 통로). 경보 신선도 규칙 `accounts::alert_eligible` 의 나이 입력이다.
    /// 원값은 직렬화하지 않는다 — status 에는 계산 키(`rate_observed_at`·`rate_age_secs`)만 싣는다(`accounts::seat_usage_wire`).
    #[serde(skip)]
    pub rate_observed_at: f64,
    /// ★0.14.43(B3): 이 rate 가 귀속된 계정 id — 상태줄 보고를 좌석 설정 폴더의 **보고 시점 신원**으로 귀속한 결과(claude 좌석만 ·
    /// 귀속 실패·창 밖 강등·비 claude = None). 이월이면 이월한다. 좌석 축 `seat_in_use` 의 입력("로그인이 바뀐 뒤 아직 새로 보고하지
    /// 않은 좌석" 판정). 원 uuid 라 직렬화하지 않는다(status JSON 으로 내보내지 않는다).
    #[serde(skip)]
    pub rate_account: Option<String>,
}

// ───────────────── ★(0.14.42 · clear 가드 v3) 사이클 표지 기반 유한 상태기계 — 폭주·영구 무clear·유휴 발화 차단 ─────────────────
//
// 【왜】 clear 사이클은 지침 전문을 다시 붙여 넣는다. 200K(또는 창 미상) master·CEO 는 그 붙여넣기와 복원만으로 60~78% 에 돌아와,
// 기본 임계(60)의 에지 래치만으로는 유휴 좌석이 경보 쿨다운마다 clear·재주입을 되풀이했다(①). 6cda56b0 ~ 69e3dda6 은 바닥을
// 추정·분류(영역 5종 · 정착 창 · 조용함 닫힘 · 몰림 추적 · 얇은 세션 속도 · 압축 뒤 들어올림)해 고쳤지만 고칠 때마다 새 반례가
// 나왔고, 마지막 판은 여유 좌석을 막대 101 로 영구히 잠갔다(RV3NC-1 · ②). 설계 정본:
// ceo/socket-bidir-2026-09-23/reports/final/clear-guard/clear-guard-redesign.md(v3) · 참조 구현 evidence/v3/model/src/cg3.rs.
//
// 【무엇】 좌석마다 상태 넷(Free · Awaiting · Cycling · Measuring)과 입력 넷뿐이다 — `report`(관측 1건: 퍼센트·창·축·범위·기본
// 임계·동결) · `cycle`(사이클 표지 = cys cycle-agent 의 quiescing 켬/끔 · `surface.quiesce` · `governance::release_quiescing`)
// · `tick`(수집기 2초) · `stale`(집행자 단일 비행 질의). **세션 키는 입력이 아니다** — clear 의 유일한 증거는 사이클 표지이고,
// 세션 파일 줄기는 낙폭(압축) 판정의 범위 이름일 뿐이다(헬퍼·휴리스틱 재발견·빈 경로가 수준·막대·발화에 닿는 길이 없다).
//   · 표지 끔은 집행자가 아는 결과를 싣는다(`Outcome` — clear 실효 확인 · clear 안 됨 · 모름). **clear 안 됨**(표지 켬 뒤 송신
//     거부 · 실효 미관측)은 사이클 끝이 아니다 — 가드는 표지 켬 전 상태로 돌아가고(시한·재는 창 그대로) 그 발화는 미해결로 남는다
//     (같은 --fire 재집행 가능 · 시한이 지나면 효과 없음). 확인·모름은 사이클 끝이다 — 발화를 풀고 · 앞선 효과 없음 보류를 끝내고
//     (표지 = 집행자가 응답했다는 증거 · strikes 는 다음 발화가 정한다) · 수준을 다시 잰다(모름은 feed 가 처방을 싣지 않는다).
//   · 표지 끔(재주입 직후)부터 600초 동안의 최고치를 **축별**(실측 = 상태줄·transcript·rollout / 자기보고 = status.set) 수준 R 로
//     잰다. 창이 시각으로 닫힌 뒤 그 축의 첫 관측이 **S 미만이면** R 에 접는다(창 끝의 관측 공백을 메운다 · 그 관측은 발화하지
//     않는다). **S 이상이면 접지 않는다** — 접기 기회만 소비하고 접기 전 막대로 곧바로 판정한다(수정 4회차 RNC6-1 · 자동 압축을 끈
//     200K 좌석에서 창보다 긴 간격의 덩어리가 S 를 넘은 보고를 접으면 다음 덩어리가 차단점을 한 번에 넘는다 · ②). 창 안에서
//     낙폭(압축)이 오면 창을 그때까지의 최고치로 먼저 닫는다(잰 사실을 버리지 않는다).
//   · **S 가장자리**: 사이클 뒤 창 안의 **확인된** S 이상 관측(같은 범위의 S 미만 최고치가 창에 있다 · 또는 같은 범위의 두 번째
//     S 이상 보고 = 외톨이 짝)은 창을 곧바로 닫고 **그 보고로 곧바로 판정한다**(그 보고는 R 밖 · 짝의 낮은 값도 R 밖 · 막대 ≤ S —
//     RNC5-1 · 수정 6회차 V42NC-1). 자동 압축이 꺼진 200K 좌석은 S→차단점 여유가 3.5%p(7K 토큰)라 작업 한 덩어리보다 작을 수 있다 — 그
//     보고를 R 에 접고 R+1 을 기다리면 다음 덩어리가 차단점을 한 번에 넘고 저장 지시가 거부돼 끝까지 막힌다(②). 대가: 사이클 뒤
//     10분 안에 S 로 돌아오는 좌석은(복원·붙잡혔던 회신만으로도) 사이클마다 한 번 발화한다 — 복원으로 돌아온 좌석과 작업으로
//     돌아온 좌석은 **같은 관측값 열**이라 가드가 가를 수 없다(창 안 관측의 시각은 판정 입력이 아니다 · 검체
//     `edge_cannot_tell_restore_from_work_so_both_fire_every_cycle`). 이 고리는 최소 간격에 묶이고(≤ 6/시간 · 24시간 ≤ 144 ·
//     차단점 위 0초) 오너가 결재한다(기본값 ② 우선 — 수정 4회차). 가드는 그 발화를 **가장자리·C 띠 복귀 발화**로 세어(연속 수
//     `edge_run` · 고리 회차 `edge_episode` · 24시간 복귀·사이클 수) 오너 error feed 를 **고리 회차**의 1·2·4·8·16·32·64·128…번째에
//     낸다(수정 5회차 R1V4-1 — 회차는 24시간 공백 뒤에만 새로 센다 · 좌석당 24시간 ≤ 8건 · 6시간 제한 밖 · 처방 후보: 자동 압축 켬 ·
//     1M · 지침 축소 · 자기보고면 자기보고 정정·실측 축 확보).
//   · **고착 자기보고**(수정 4회차 R3V3-1 ③ · 수정 5회차 V41NC-1 범위): 자기보고 축 발화 뒤 끝난 사이클의 그 축 첫 보고가 발화 때와
//     **정확히 같고 차단점 이상으로만 보이는 값**(B · 200K·미상 89 · 1M 99)이면 그 사이클은 그 축에서 효과 증거가 없다 — 효과 없음
//     1회(strike · 900·2^(k−1) ≤ 7200). 값이 바뀌거나 B 아래면 정상 규칙 — B 아래(C 이하)는 차단점 전의 좌석이 보일 수 있는 값이라
//     창보다 드문 정직한 보고의 재성장과 같은 관측이다(S 아래는 고리가 없고 · S~C 는 ② 우선 — C 를 고착으로 세면 보류 동안 차단점을
//     넘은 정직한 좌석이 보고조차 못 해 영구 무clear 였다 · 060075e8). 실측 축은 쓰지 않는다(관측 동치 — 실측 C 띠·가장자리 좌석은
//     ② 우선으로 최소 간격 고리를 유지한다).
//   · 발화 막대 = max(기본, min(R+5, max(S, R+1), C)) — S = 창 − 28K 토큰(200K 85 · 1M 96) · C = 창 − 23K 토큰(차단점 아래로
//     보이는 가장 큰 정수 · 200K 88 · 1M 97). 막대는 C 를 넘지 않는다(101·NEVER 없음).
//   · 같은 축·같은(확인된) 범위에서 10%p 이상 떨어지면 압축 — 90초 동안 다시 잰다(자동·수동 /compact·수동 /clear 구분 불가).
//   · 연속 두 발화 사이 최소 600초(굴림 1시간 ≤ 6). 효과 없는 발화(사이클 뒤 다음 발화 전 압축 · 표지 없이 다시 발화)는
//     **발화 1건당 한 번만** 세고 15·30·60·120분(상한)으로 미룬다. 사이클 전 압축은 효과 없음이 아니다. 효과 없음의 오너 feed 는
//     **효과 없음 회차**(24시간 공백 뒤에만 새로 셈 · 효과 있는 발화로 끊기지 않음)의 1·2·4·8…번째에만 낸다(좌석당 24시간 ≤ 8 ·
//     통합 minor 정리 R1V42-3 · 발행 재료 — 판정 무관).
//   · 발화 → 표지 켬 시한 1200초는 잠정이다(늦은 표지도 그 발화의 clear) · 배달 동결(system.pause) 중에는 흐르지 않는다.
//   · 틱은 가드 밖 값(observed_usage·agent_status·오버라이드 파일)을 읽지 않는다 — 시한 처리와 보류 만료 재판정뿐이다. 재판정은
//     보류해 둔 값이 아니라 **그 축의 가장 최근 관측**(가드가 받은 마지막 실측 보고 · 범위 무관)이다(RR2-ROLE-1 — 표지 없는
//     clear 뒤 새 세션이 막대 아래면 죽은 세션의 값을 되살리지 않는다 · 보고가 끊긴 좌석은 마지막 관측으로 재통보 — I2).
//   · 집행 시도가 사이클을 끝내지 못하고 물러나면(단일 비행 점유 해제 · 점유자 사망) 아직 미해결(Awaiting)인 그 발화를 **한 번**
//     다시 알린다(`redeliver` — 같은 발화 번호 · 새 발화 아님 · RR2-ROLE-2).
//
// 【불변식(좌석 · 데몬 세대당)】
//   (I1 폭주 없음) 발화를 만드는 코드는 `decide` 하나이고 `now ≥ last_fire + 600` 일 때만 Fire → 굴림 1시간 ≤ 6. `decide` 에
//        이르는 길은 게이트(`report` — 세 보고 경로 공용 · 가드 락 하나 안)와 틱(보류 재판정)뿐이다. 재배달(`redeliver`)은 발화가
//        아니다(같은 번호 · 발화 1건당 최대 1회). 집행 수는 단일 비행 `stale` 이 묶되 **발화 1건당 사이클은 최대 2회**다 — clear 안 됨
//        (rc 80·85) 끔은 통보를 풀지 않으므로 그 뒤 재배달·같은 `--fire` 재집행이 한 번 더 돌 수 있다(rc 80 = clear 는 나갔으나 실효
//        미관측 — 늦게 발효한 clear 뒤 좌석에 지침·재개 포인터를 넣는 의도된 복구 · 통합 minor 정리 R3V3-4 문구 정정). CSO 의
//        `--detach` 는 자기 실패 뒤 같은 통보의 재접수를 데몬이 repeat 87 로 막는다(수정 6회차).
//   (I2 영구 무clear 없음) 비-Free 상태는 시각만으로 끝난다(Awaiting ≤ 1200(+동결) · Cycling ≤ 660 · Measuring ≤ 600/90) ·
//        Free 에서 막대 이상인 관측은 Fire 또는 Held{until − now ≤ 7200} · 막대 ≤ max(기본, C) < 차단점 · 사이클 뒤 창 안의 확인된
//        S 이상 관측(외톨이 짝 포함 — V42NC-1)은 창을 기다리지 않고 판정된다(막대 ≤ S) · 창 뒤 첫 관측도 S 이상이면 접지 않고
//        판정된다(RNC6-1).
//   (I3 유휴 무발화) 발화 ⇒ pct ≥ bar ≥ min(R+5, max(S, R+1), C) — 끝난 모든 cys 사이클이 R 을 다시 잰다(R = 창 안의 S 미만
//        관측 · 창 뒤 첫 S 미만 관측 · S 이상 관측은 R 에 들지 않는다). 예외(명시): ⓐ 세대 첫 관측 · ⓑ S 가장자리 — 사이클 뒤
//        창 안의 확인된 S 이상 관측(단일 확인 · 외톨이 짝 — V42NC-1)과 창 뒤 첫 S 이상 관측(그 보고는 R 밖 → 사이클 뒤 창 안·창 끝
//        첫 보고로 S 에 돌아오는 좌석은 사이클마다 1회 · ≤ 6/시간) · ⓒ C 띠 — R ≥ C 이면 막대 = C ≤ R(사이클 뒤 수준이 C 이상인 좌석은 최소 간격마다 · ≤ 6/시간)
//        — 차단점 이상으로만 보이는 값(B · 200K 89)을 상수로 보고하는 **자기보고** 좌석은 고착 자기보고 strike 가 백오프로 묶는다(24시간
//        ≤ 15 · C(88) 상수는 정직한 재성장과 관측 동치라 ⓒ 고리 그대로 ≤ 144 · 가장자리 띠 85~87 을 창보다 드물게 보고하는 자기보고
//        좌석은 ⓑ 로 보고 간격마다 — 900초면 24시간 96 · 간격이 창 끝 바로 뒤면 보고마다 ≤ 144) · ⓓ 표지 없는 clear
//        (오너 수동 /clear · 재기동)는 R 을 다시 재지 않는다 — 좌석의 가장 최근 관측이 그 막대 이상일 때만 발화 · ⓔ 자기보고 표본
//        착오. ⓑ·ⓒ 는 자동 압축이 꺼진 좌석(또는 C 이상을 보고하는 좌석)의 I2 대가다(clear 로 차단점 아래를 유지할 수 없는 좌석 —
//        고리 회차 1·2·4·8…번째 오너 error feed · 오너 결정).
// 가드 상태는 휘발이다 — 데몬 재기동 뒤 각 축 첫 교차는 기본 임계에서 1회 발화한다(가드 전 f7f7a262 과 같다 · 실패 방향 = 발화).

/// 잰 수준 위로 이만큼(%p · 표시값) 자라야 발화한다(G) — 표시 5%p = 참 성장 4%p 이상(반올림 두 번).
pub const CTX_GUARD_GROWTH: u8 = 5;
/// 사이클 표지 끔(재주입 직후)부터 수준을 재는 창(W · 초).
pub const CTX_GUARD_MEASURE_SECS: f64 = 600.0;
/// 압축(같은 범위 10%p 낙폭) 뒤 수준을 재는 창(초) — SessionStart:compact 훅이 시키는 지침 재읽기 몫.
pub const CTX_GUARD_REREAD_SECS: f64 = 90.0;
/// 압축으로 보는 같은 축·같은 범위의 낙폭(%p).
pub const CTX_GUARD_COMPACT_DROP: u8 = 10;
/// 연속 두 발화 사이 최소 간격(초) — I1 상한 그 자체(굴림 1시간 ≤ 6).
pub const CTX_GUARD_MIN_SPACING_SECS: f64 = 600.0;
/// 효과 없는 발화 k 번 연속 뒤 보류 = min(BASE · 2^(k−1), MAX)(초) — 7200 = I2 의 T 상한.
pub const CTX_GUARD_BACKOFF_BASE_SECS: f64 = 900.0;
pub const CTX_GUARD_BACKOFF_MAX_SECS: f64 = 7200.0;
/// 발화 → 사이클 표지 켬 시한(초) — 라우터 유예·쿨다운 ≤300 + 사이클 1~4단계 ≤360 + 픽업 여유 540. 잠정 · 동결 중 정지.
pub const CTX_GUARD_CLEAR_WAIT_SECS: f64 = 1200.0;
/// 표지 켬 뒤 끔이 오지 않을 때의 상한(초) = quiescing 상한(`CYS_QUEUE_QUIESCE_HOLD_SECS` 기본 600) + 60.
pub const CTX_GUARD_CYCLING_MAX_SECS: f64 = 660.0;
/// S = 창 − (요약 예약 + 차단 버퍼 + 이것) 토큰 — 이 높이부터는 G 성장 없이 1%p 로 발화한다(자동 압축을 끈 좌석).
pub const CTX_GUARD_STOP_MARGIN_TOKENS: u64 = 5_000;
/// 축당 낙폭 판정 범위 보관 수 — 헬퍼·휴리스틱 교대 A→H→A 에서 좌석 범위의 최고치를 잃지 않는다.
pub const CTX_GUARD_SCOPES_KEPT: usize = 4;
/// Claude Code 요약 출력 예약 상한 — 유효 창 = 창 − min(최대 출력, 이 값). 상한을 쓴다(보수 — 실제 예약은 이하).
pub const CC_SUMMARY_RESERVE_TOKENS: u64 = 20_000;
/// Claude Code 자동 압축 버퍼 — 선제 압축점 = 유효 창 − 이 값(feed 문구 전용).
pub const CC_AUTOCOMPACT_BUFFER_TOKENS: u64 = 13_000;
/// Claude Code 차단 버퍼 — 차단점 = 유효 창 − 이 값(자동 압축 끔·비-auto 압축 창).
pub const CC_BLOCKING_BUFFER_TOKENS: u64 = 3_000;
/// 창을 모를 때 가정하는 창 — claude 최소 창(200K). 작은 창을 가정해야 S·C 가 낮아져 발화 쪽으로 실패한다.
pub const CTX_ASSUMED_WINDOW: u64 = 200_000;

/// (창 − 예약)으로 **보이는** 가장 큰 정수 퍼센트 — 상태줄 `used_percentage` 는 반올림이라 c% 는 (c+0.5)% 직전까지다.
/// c ≤ (200·(창 − 예약) − 창) / (2·창). 창 미상(None·0)은 [`CTX_ASSUMED_WINDOW`]. 창이 예약보다 작으면 0.
pub(crate) fn ctx_pct_below_reserve(window: Option<u64>, reserve: u64) -> u8 {
    let w = window.filter(|w| *w > 0).unwrap_or(CTX_ASSUMED_WINDOW);
    let usable = w.saturating_sub(reserve);
    let pct = usable.saturating_mul(200).saturating_sub(w) / w.saturating_mul(2);
    pct.min(100) as u8
}

/// (창 − 예약) **이상으로만** 보이는 가장 작은 정수 퍼센트 — c% 는 (c−0.5)% 부터라 c ≥ (창 − 예약)/창·100 + 0.5.
/// c = ⌈(200·(창 − 예약) + 창) / (2·창)⌉. [`ctx_pct_below_reserve`] 의 짝이다(그 값 + 1 이거나 + 2 — 경계가 반올림 칸 가운데를 지나면
/// 두 칸 사이의 한 표시값이 경계 양쪽에 걸친다 · 1M 차단점 97.7 → 98 이 그렇다). 창 미상은 [`CTX_ASSUMED_WINDOW`]. 100 캡.
pub(crate) fn ctx_pct_at_or_above_reserve(window: Option<u64>, reserve: u64) -> u8 {
    let w = window.filter(|w| *w > 0).unwrap_or(CTX_ASSUMED_WINDOW);
    let usable = w.saturating_sub(reserve);
    let pct = usable.saturating_mul(200).saturating_add(w).div_ceil(w.saturating_mul(2));
    pct.min(100) as u8
}

/// clear 가드 v3 — 순수 상태기계(시각은 호출자가 데몬 단조 초로 준다 · 락·I/O 없음). 참조 구현 cg3.rs 를 파라미터만 상수로
/// 옮겼다(발화 판정에 쓰이지 않는 발행 재료 몇 개 — `Verdict::Fire` 의 축·기본·창 · `Note::Ineffective` 의 발화 번호·시각 —
/// 만 더했다).
pub(crate) mod clear_guard {
    use super::{
        ctx_pct_at_or_above_reserve, ctx_pct_below_reserve, CC_BLOCKING_BUFFER_TOKENS, CC_SUMMARY_RESERVE_TOKENS,
        CTX_GUARD_BACKOFF_BASE_SECS, CTX_GUARD_BACKOFF_MAX_SECS, CTX_GUARD_CLEAR_WAIT_SECS, CTX_GUARD_COMPACT_DROP,
        CTX_GUARD_CYCLING_MAX_SECS,
        CTX_GUARD_GROWTH, CTX_GUARD_MEASURE_SECS, CTX_GUARD_MIN_SPACING_SECS, CTX_GUARD_REREAD_SECS, CTX_GUARD_SCOPES_KEPT,
        CTX_GUARD_STOP_MARGIN_TOKENS,
    };

    /// S — 이 높이부터는 G 성장 없이 1%p 로 발화한다(200K 85 · 1M 96 · 미상 85).
    pub fn stop_cap(window: Option<u64>) -> u8 {
        ctx_pct_below_reserve(window, CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS + CTX_GUARD_STOP_MARGIN_TOKENS)
    }

    /// C — 막대의 절대 상한 = 차단점(창 − 23K) 아래로 보이는 가장 큰 정수(200K 88 · 1M 97 · 미상 88). 막대가 이 위로 가지
    /// 않으므로 자동 압축을 끈 좌석도 막대에 닿을 수 있다.
    pub fn block_cap(window: Option<u64>) -> u8 {
        ctx_pct_below_reserve(window, CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS)
    }

    /// ★(수정 5회차 · V41NC-1) B — 차단점(창 − 23K) **이상으로만** 보이는 가장 작은 표시값(200K 89 · 1M 99 · 미상 89). 차단점 전의
    /// 좌석은 이 값을 보일 수 없다(C 이하 · 1M 은 98 까지) — 고착 자기보고 규칙의 하한이다: 사이클 전후 같은 값이 이 아래면 정직한
    /// 재성장(창보다 드문 보고가 복원 뒤 같은 값에 닿음)과 관측이 같다.
    pub fn blocked_floor(window: Option<u64>) -> u8 {
        ctx_pct_at_or_above_reserve(window, CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS)
    }

    /// (퍼센트, 그 창).
    pub type Lv = (u8, Option<u64>);

    /// 퍼센트를 창 `to` 기준으로 옮긴다(토큰 비율 · 올림 · 100 캡). 창이 같거나 한쪽이 미상이면 그대로.
    pub fn rebase(v: Lv, to: Option<u64>) -> u8 {
        match (v.1, to) {
            (Some(fw), Some(w)) if fw != w && w > 0 => (u64::from(v.0).saturating_mul(fw).div_ceil(w)).min(100) as u8,
            _ => v.0,
        }
    }

    fn higher(a: Lv, b: Lv) -> Lv {
        if rebase(a, b.1) >= b.0 {
            (rebase(a, b.1), b.1)
        } else {
            b
        }
    }

    /// 발화 막대 = max(기본, min(R+G, max(S, R+1), C)) — R 미상이면 기본. ≤ C(≤ 97)이거나 기본. R 은 창 `window` 로 옮겨 잰다.
    /// S 부터는 G 대신 1%p 성장이면 발화한다(가장자리 — 자동 압축을 끈 좌석이 차단점 전에 통보받는다). R < C 이면 막대 > R
    /// (성장 없이는 발화하지 않는다) · R ≥ C 이면 막대 = C(설계 I3 예외 — C 이상으로 돌아오는 좌석).
    pub fn bar_of(level: Option<Lv>, base: u8, window: Option<u64>) -> u8 {
        match level {
            Some(l) => {
                let r = rebase(l, window);
                let grown = r.saturating_add(CTX_GUARD_GROWTH);
                let edge = stop_cap(window).max(r.saturating_add(1));
                base.max(grown.min(edge).min(block_cap(window)))
            }
            None => base,
        }
    }

    /// 효과 없는 clear 연속 k 번 뒤 보류(초) = min(BASE · 2^(k−1), MAX).
    pub fn backoff(k: u32) -> f64 {
        if k == 0 {
            return 0.0;
        }
        (CTX_GUARD_BACKOFF_BASE_SECS * 2f64.powi(k.min(16) as i32 - 1)).min(CTX_GUARD_BACKOFF_MAX_SECS)
    }

    /// 관측 축. 실측(상태줄·transcript·rollout) · 자기보고(status.set). 수준·낙폭 판정은 축 안에서만 한다.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Axis {
        Measured = 0,
        SelfReport = 1,
    }
    pub const AXES: [Axis; 2] = [Axis::Measured, Axis::SelfReport];

    impl Axis {
        pub fn as_str(self) -> &'static str {
            match self {
                Axis::Measured => "measured",
                Axis::SelfReport => "self_report",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        AfterCycle,
        AfterCompaction,
    }

    impl Kind {
        pub fn as_str(self) -> &'static str {
            match self {
                Kind::AfterCycle => "after_cycle",
                Kind::AfterCompaction => "after_compaction",
            }
        }
    }

    /// 사이클 표지 끔이 싣는 결과 — 집행자(`cys cycle-agent`)가 아는 사실이다(가드는 추측하지 않는다).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Outcome {
        /// clear 실효가 확인됐다(session_file 교체 — cycle-agent rc 0 · 86).
        Cleared,
        /// clear 가 나가지 않았거나(표지 켬 뒤 송신 거부 · rc 85) 실효가 관측되지 않았다(rc 80) — 사이클 끝이 아니다.
        NotCleared,
        /// 모른다 — 실효 측정 불능(rc 81) · 데몬 해제(세운 호출자 사망 · 에이전트 종료·재기동) · 수동 `cys quiesce` · 결과를
        /// 싣지 않는 구 CLI. 사이클 끝으로 본다(종전과 같다) — feed 는 clear 를 단정하지 않는다.
        Unknown,
    }

    impl Outcome {
        /// `surface.quiesce{on:false, outcome}` 의 값 — 모르는 값·부재는 Unknown(종전 동작 · 실패 방향 = 수준 재측정).
        pub fn parse(s: Option<&str>) -> Self {
            match s.map(str::trim) {
                Some("cleared") => Outcome::Cleared,
                Some("not_cleared") => Outcome::NotCleared,
                _ => Outcome::Unknown,
            }
        }
        pub fn as_str(self) -> &'static str {
            match self {
                Outcome::Cleared => "cleared",
                Outcome::NotCleared => "not_cleared",
                Outcome::Unknown => "unknown",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Default)]
    pub enum Phase {
        #[default]
        Free,
        /// 발화했다 — 사이클 표지(quiescing 켬)를 `deadline` 까지 기다린다.
        Awaiting { at: f64, deadline: f64 },
        /// 사이클 표지 켬 — clear·복원이 도는 중(관측은 판정·낙폭에 쓰지 않는다).
        Cycling { since: f64 },
        /// 사이클 끝·압축 뒤 [anchor, anchor+secs) 동안 축별 최고치를 잰다.
        /// `hi` = 확인 전 S 이상 외톨이 보고(값 · 범위) · `peak_scope` = 최고치를 낸 범위 · `confirmed` = 이 창을 연 사이클의 clear
        /// 실효가 확인됐다(`Outcome::Cleared` — feed 문구 전용 · 판정에 쓰지 않는다).
        Measuring {
            anchor: f64,
            secs: f64,
            kind: Kind,
            peak: [Option<Lv>; 2],
            hi: [Option<(Lv, u64)>; 2],
            first: [Option<Lv>; 2],
            peak_scope: [u64; 2],
            confirmed: bool,
        },
    }

    impl Phase {
        pub fn as_str(&self) -> &'static str {
            match self {
                Phase::Free => "free",
                Phase::Awaiting { .. } => "awaiting",
                Phase::Cycling { .. } => "cycling",
                Phase::Measuring { .. } => "measuring",
            }
        }
    }

    /// 범위 이름의 지문(FNV-1a · 0 은 '범위 없음'으로 쓰지 않도록 1 로 올린다).
    pub fn scope_id(s: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h.max(1)
    }

    /// 효과 없음의 사실(오너 feed 문구가 원인을 단정하지 않게 둘로 나눈다).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Why {
        /// 발화 뒤 사이클이 끝났는데 다음 발화 전에 같은 범위에서 10%p 이상 떨어졌다(압축 — 자동·수동 구분 불가).
        DropAfterCycle,
        /// 직전 발화 뒤 사이클 표지가 한 번도 관측되지 않은 채 다시 발화 조건이 됐다(원인은 가드가 모른다).
        NoCycle,
        /// ★(수정 4회차 · R3V3-1 ③ · 수정 5회차 V41NC-1 범위) 자기보고 축 발화 뒤 사이클이 끝났는데 **사이클 뒤 첫 자기보고가 발화 때와
        /// 정확히 같고 차단점 이상으로만 보이는 값**([`blocked_floor`] 이상 · 200K 89)이다 — 그 사이클의 효과를 그 축에서 볼 수 없다
        /// (고착 자기보고 · 원인은 가드가 모른다 — 갱신되지 않는 자기보고 · 정말로 clear 로 낮출 수 없는 수준). C 이하는 쓰지 않는다
        /// (정직한 재성장과 관측 동치 · ②). 실측 축에는 쓰지 않는다(관측 동치 — 복원으로 돌아온 좌석과 작업 좌석이 같은 값 열이다 · ②).
        SelfReportUnchanged,
    }

    impl Why {
        pub fn as_str(self) -> &'static str {
            match self {
                Why::DropAfterCycle => "drop_after_cycle",
                Why::NoCycle => "no_cycle",
                Why::SelfReportUnchanged => "self_report_unchanged",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Note {
        /// 재는 창이 닫혔다 — 축별 새 수준(None = 창 안 관측 없음 → 그 축의 다음 관측이 수준) · 창의 첫 관측 · 최고치를 낸 관측의
        /// 기본 임계(발행 재료 — 틱이 오버라이드 파일을 다시 읽지 않게) · `confirmed` = 창을 연 사이클의 clear 실효 확인 ·
        /// `span` = 창이 실제로 잰 초(시각으로 닫히면 창 길이 · S 도달·낙폭이면 그때까지) · `cut` = 낙폭(압축)이 창을 끊었다 ·
        /// `edge` = 창을 S 가장자리에서 닫은 보고(그 보고는 수준 밖 · 곧바로 판정된다 — feed 가 사이클 뒤 S 복귀를 싣는다) ·
        /// `at` = 창이 닫힌 시각(가드 단조 초 = 창 시작 + span · feed 가 같은 Out 의 보류까지 남은 초를 이 시각 기준으로 적는다).
        Measured {
            kind: Kind,
            level: [Option<Lv>; 2],
            first: [Option<Lv>; 2],
            base: [Option<u8>; 2],
            confirmed: bool,
            span: f64,
            cut: bool,
            edge: Option<Lv>,
            at: f64,
        },
        /// 창이 닫힌 뒤 그 축의 첫 관측을 수준에 접었다(이 관측은 발화하지 않는다).
        MeasuredLate { axis: Axis, level: Lv },
        /// 효과 없는 발화 — 연속 `strikes` 번째 · 다음 발화는 `hold_until` 뒤. `at` = 판정 시각 · `fire_seq`·`fire_at` = 그 발화 ·
        /// `level` = 그 축의 잰 수준(발행 재료) · `drop` = (전, 후) — 낙폭이면 그 두 값 · 고착 자기보고면 (발화 값, 사이클 뒤 첫
        /// 자기보고 값 — 같다). ★(통합 minor 정리 · R1V42-3 · 발행 재료) `episode` = **효과 없음 회차** — 이 좌석의 효과 없음 누적 수
        /// (직전 효과 없음이 24시간보다 오래됐을 때만 1 부터 다시 센다 · 효과 있는 발화로 끊기지 않는다 — 연속 수 `strikes` 는 끊긴다) ·
        /// `ineffective_24h` = 24시간 효과 없음 수(이것 포함). 오너 feed 는 회차의 2의 거듭제곱 번째에만(좌석당 24시간 ≤ 8 — 고리 회차와
        /// 같은 증명 · 백오프가 연속 효과 없음을 더 드물게 한다).
        Ineffective {
            strikes: u32,
            episode: u32,
            ineffective_24h: u32,
            hold_until: f64,
            at: f64,
            why: Why,
            drop: Option<(u8, u8)>,
            fire_seq: u64,
            fire_at: f64,
            level: Option<Lv>,
        },
        /// 발화 뒤 사이클 전에 낙폭(압축)이 왔다 — 효과 없음이 아니다(발화 1건당 결과 1회 · strike 없음).
        DropBeforeCycle { seq: u64, drop: (u8, u8) },
        /// 발화 뒤 시한까지 사이클 표지가 없었다(잠정 — 이것만으로 효과 없음을 세지 않는다).
        Unanswered { seq: u64, retry_after: f64 },
        /// ★(수정 4회차 · R3V3-1 ①②) 이 발화는 직전 발화 뒤 끝난 cys 사이클 뒤 잰 수준 위 G 성장을 기다리지 않는 예외로 났다 —
        /// 사이클 뒤 창을 S 가장자리에서 닫은 보고 · 창 뒤 첫 관측이 S 이상(예외 ⓑ) · 또는 잰 수준이 C 이상(막대 = C · 예외 ⓒ). 판정에 쓰지 않는다(발행 재료 — 오너 feed · 좌석 행).
        /// `run` = 연속 수(성장으로 난 발화가 0 으로 끊는다 · 정보) · ★(수정 5회차 R1V4-1) `episode` = **고리 회차** — 이 좌석의 복귀
        /// 발화 누적 수(직전 복귀 발화가 24시간보다 오래됐을 때만 1 부터 다시 센다 · 성장 발화로 끊기지 않는다 · 오너 error feed 는 이
        /// 값이 2의 거듭제곱일 때만 — 어느 24시간 창도 한 회차의 연속 구간만 닿고 I1 이 그 안의 복귀 발화를 ≤ 144 로 묶으므로 ≤ 8건) ·
        /// `returns_24h` = 24시간 복귀 발화 수(이 발화 포함) · `cycles_24h`·`confirmed_24h` = 이 좌석의 24시간 끝난 사이클 수·그중
        /// clear 실효 확인 수 · `confirmed` = 이 발화 직전 사이클의 clear 실효 확인 · `c_band` = 잰 수준 ≥ C.
        EdgeReturn {
            run: u32,
            episode: u32,
            returns_24h: u32,
            seq: u64,
            pct: u8,
            bar: u8,
            level: Option<u8>,
            axis: Axis,
            window: Option<u64>,
            c_band: bool,
            cycles_24h: u32,
            confirmed_24h: u32,
            confirmed: bool,
        },
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Verdict {
        Quiet,
        Held { bar: u8, until: f64 },
        /// `axis`·`base`·`window`·`observed_age` 는 발행 재료(판정에 쓰지 않는다 — 틱의 보류 재판정 발화도 가드 밖 값을 다시 읽지
        /// 않고 싣는다). `observed_age` = 판정 시각에 그 관측의 나이(초 · 보고 판정이면 0 · 보류 재판정이면 그 축의 가장 최근 관측이
        /// 온 뒤 흐른 시간 — payload 의 관측 나이).
        Fire {
            pct: u8,
            bar: u8,
            level: Option<u8>,
            strikes: u32,
            seq: u64,
            after_compaction: bool,
            axis: Axis,
            base: u8,
            window: Option<u64>,
            observed_age: f64,
        },
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Out {
        pub notes: Vec<Note>,
        pub verdict: Option<Verdict>,
    }

    impl Out {
        pub fn fired(&self) -> Option<Verdict> {
            self.verdict.filter(|v| matches!(v, Verdict::Fire { .. }))
        }
    }

    /// 관측 1건.
    #[derive(Clone, Copy, Debug)]
    pub struct Rep<'a> {
        pub pct: u8,
        pub window: Option<u64>,
        pub axis: Axis,
        /// 낙폭 판정 범위(세션 파일 줄기 · 자기보고는 ""). 수준·발화에는 쓰지 않는다.
        pub scope: &'a str,
        /// 기본 임계(역할 오버라이드 · env · 60).
        pub base: u8,
        pub now: f64,
        /// 배달 동결(system.pause) — Awaiting 시한이 흐르지 않는다.
        pub frozen: bool,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct FireRec {
        pub seq: u64,
        pub at: f64,
        pub axis: Axis,
        /// 이 발화 뒤 사이클 표지(켬→끔)를 봤다.
        pub cleared: bool,
        /// 결과가 정해졌다(효과 없음 1회 · 또는 사이클 전 낙폭) — 발화 1건당 결과는 한 번뿐.
        pub evaluated: bool,
        /// 시한까지 표지가 없었을 때의 잠정 보류(표지가 늦게라도 오면 사라진다).
        pub tentative_until: Option<f64>,
        /// 발화한 관측의 범위(지문).
        pub scope: u64,
        /// 이 발화의 통보(재배달 재료 — 같은 번호·같은 사실).
        notice: Verdict,
        /// 재배달했다(발화 1건당 최대 1회).
        pub redelivered: bool,
    }

    /// 보류 — `until` 에 그 축의 **가장 최근 관측**을 다시 판정한다(보류해 둔 값이 아니다 · RR2-ROLE-1).
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Held {
        until: f64,
        axis: Axis,
    }

    /// 축의 가장 최근 관측(가드가 받아 낙폭 판정 범위에 적은 마지막 보고 — 범위 무관).
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct LastObs {
        scope: u64,
        pct: u8,
        window: Option<u64>,
        base: u8,
        at: f64,
    }

    #[derive(Clone, Debug, Default)]
    pub struct ClearGuard {
        pub phase: Phase,
        /// 축별 잰 수준(R) · 그 창의 종류.
        pub level: [Option<(Lv, Kind)>; 2],
        /// 창이 닫힌 뒤 그 축의 첫 관측(창 최고치를 낸 범위의 것 · 창이 비었으면 아무 범위)을 수준에 접는다
        /// (R := max(R, 그 관측) · 창이 비었으면 그 관측이 R) — 그 관측은 발화하지 않는다. 창 끝 무렵 관측이 끊겨(도구 호출·
        /// 상태줄 공백) R 이 참 최고치보다 낮게 잡히는 것을 막는다. Some(0) = 아무 범위 · Some(h) = 그 범위.
        pub fold: [Option<u64>; 2],
        /// 축별 낙폭 판정 범위(최근 4개 · 범위 이름 → 최고치 · 확인됨 · 최근 관측). 범위의 첫 관측은 두 번째 관측이 10%p
        /// 안에서 확인하기 전에는 낙폭 기준이 아니다(새 세션 첫 보고가 낡은 값이어도 다음 보고를 '압축'으로 오인하지 않는다).
        scopes: [Vec<(String, Lv, bool, (u8, Option<u64>, u8))>; 2],
        pub strikes: u32,
        pub hold_until: f64,
        pub last_fire: Option<f64>,
        pub fire: Option<FireRec>,
        pub seq: u64,
        /// 이 번호 이하의 발화는 그 뒤 사이클이 이미 끝났다 — 집행자는 건너뛴다(중복 사이클 방지). 사이클 전 낙폭(압축)은
        /// 건너뛰는 사유가 아니다 — 늦게라도 온 사이클은 압축된 좌석을 비우고 수준을 다시 잰다.
        pub resolved_through: u64,
        held: Option<Held>,
        /// 축별 가장 최근 관측 — 보류 재판정의 재료.
        last_obs: [Option<LastObs>; 2],
        /// 표지 켬 직전의 상태 — clear 안 됨(`Outcome::NotCleared`) 끔이면 여기로 돌아간다(시한·재는 창 그대로).
        before_cycle: Phase,
        /// (발행 전용 · 판정 밖) 이 좌석의 `context.level_measured` 오너 feed 를 마지막으로 낸 단조 시각 — 좌석당 6시간 1회.
        pub level_fed_at: Option<f64>,
        /// ★(수정 4회차 · R3V3-1 ③) 자기보고 축 발화의 사이클 뒤 첫 자기보고 대조 — (발화 번호, 발화 퍼센트, 창). 끝난 사이클이 세우고
        /// 그 축의 다음 보고·다음 발화가 지운다.
        sr_probe: Option<(u64, u8, Option<u64>)>,
        /// ★(수정 4회차 · R3V3-1 ①) 축별 — 사이클 뒤 창을 S 가장자리에서 닫았다 · 또는 창 뒤 첫 관측이 S 이상이었다(그 축의 다음
        /// 발화가 가장자리 복귀 발화 · 다른 창이 닫히거나 그 축의 막대 아래 판정이 지운다 — 수정 5회차 V41R-2). ★(통합 minor 정리 ·
        /// R1V42-1) 값은 그 가장자리 보고의 **범위**다 — 같은 범위의 막대 아래 판정만 지우고, 같은 범위의 관측으로 난 발화만 복귀로
        /// 센다(헬퍼 세션의 낮은 상태줄 한 건이 본체의 가장자리 표시를 지워 고리가 복귀 계수·feed·좌석 행에서 사라지던 것 · 표지 없는
        /// clear 뒤 새 세션(다른 범위)의 성장 발화는 여전히 복귀가 아니다).
        edge_mark: [Option<u64>; 2],
        /// ★(게이트 수정 1회차 ROLE-G2 · 발행 전용) 축별 — 이 사이클 뒤 창(또는 창 뒤 첫 관측)에서 **예외 ⓑ 가장자리 관측이 있었던 범위**
        /// (`edge_mark` 와 같이 선다). `edge_mark` 와 달리 막대 아래 판정이 지우지 않는다 — 같은 범위가 가장자리에 닿은 뒤 1~2%p 아래로
        /// 흔들렸다 다시 난 발화(자기보고 86/84)를 복귀로 세는 재료다. 새 창(`close`)과 발화가 지운다. 가장자리 관측이 없던 사이클(창이
        /// S 아래에서 시각으로 닫히고 작업 성장으로 막대 = max(S, R+1) 에 닿은 발화 — 사이클 뒤 81~84)은 여기 없다(복귀 아님).
        edge_seen: [Option<u64>; 2],
        /// (발행 전용) 연속 가장자리·C 띠 복귀 발화 수 — 사이클 뒤 성장으로 난 발화가 0 으로 끊는다(사이클 없이 난 발화는 그대로 ·
        /// 좌석 행 `edge_run` · 정보 — feed 조절에 쓰지 않는다: 성장 발화와 번갈면 늘 1 이다 · R1V4-1).
        pub edge_run: u32,
        /// ★(수정 5회차 · R1V4-1 · 발행 전용) 고리 회차 — 복귀 발화 누적 수(직전 복귀 발화가 24시간 안에 없을 때만 새로 센다). 오너
        /// error feed 는 이 값의 2의 거듭제곱 번째에만(좌석 행 `edge_episode`).
        pub edge_episode: u32,
        /// ★(수정 5회차 · R1V4-1 · 발행 전용) 복귀 발화 시각 — 24시간치(좌석 행 `edge_returns_24h` · feed · 회차 재시작 판정).
        edge_log: std::collections::VecDeque<f64>,
        /// (발행 전용) 끝난 사이클의 (시각, clear 실효 확인) — 24시간치(좌석 행 `clears_24h` · feed).
        cycle_log: std::collections::VecDeque<(f64, bool)>,
        /// ★(통합 minor 정리 · R1V42-3 · 발행 전용) 효과 없음 회차 — 효과 없음 누적 수(직전 효과 없음이 24시간 안에 없을 때만 새로
        /// 센다). 오너 feed(`context.clear_ineffective`)는 이 값의 2의 거듭제곱 번째에만(좌석 행 `ineffective_episode`).
        pub ineff_episode: u32,
        /// ★(통합 minor 정리 · R1V42-3 · 발행 전용) 효과 없음 시각 — 24시간치(좌석 행 `ineffective_24h` · 회차 재시작 판정).
        ineff_log: std::collections::VecDeque<f64>,
    }

    impl ClearGuard {
        /// 발화 막대(축별) = [`bar_of`] — R 미상이면 기본. ≤ C(≤ 97)이거나 기본.
        pub fn bar(&self, axis: Axis, base: u8, window: Option<u64>) -> u8 {
            bar_of(self.level[axis as usize].map(|(l, _)| l), base, window)
        }

        /// 최근 `secs` 초 안에 끝난 사이클 수 · 그중 clear 실효 확인 수(발행 재료 — 판정에 쓰지 않는다).
        pub fn cycles_within(&self, now: f64, secs: f64) -> (u32, u32) {
            self.cycle_log.iter().filter(|(t, _)| now - *t <= secs).fold((0, 0), |(n, c), (_, ok)| (n + 1, c + u32::from(*ok)))
        }

        /// 최근 `secs` 초 안의 가장자리·C 띠 복귀 발화 수(발행 재료 — 판정에 쓰지 않는다 · 수정 5회차 R1V4-1).
        pub fn edge_returns_within(&self, now: f64, secs: f64) -> u32 {
            self.edge_log.iter().filter(|t| now - **t <= secs).count() as u32
        }

        /// 최근 `secs` 초 안의 효과 없음 수(발행 재료 — 판정에 쓰지 않는다 · 통합 minor 정리 R1V42-3).
        pub fn ineffective_within(&self, now: f64, secs: f64) -> u32 {
            self.ineff_log.iter().filter(|t| now - **t <= secs).count() as u32
        }

        /// (집행 순서 재료 · 판정 밖) 발화 `seq` 의 통보 퍼센트·창 — 지금 기록된 발화가 그것일 때만(수정 6회차 V42R-1 · 비동기
        /// 사이클 대기열이 차단점까지 여유 작은 좌석을 먼저 띄운다).
        pub fn fire_notice(&self, seq: u64) -> Option<(u8, Option<u64>)> {
            self.fire.filter(|f| f.seq == seq).and_then(|f| match f.notice {
                Verdict::Fire { pct, window, .. } => Some((pct, window)),
                _ => None,
            })
        }

        /// 집행자 단일 비행: 발화 `seq` 뒤에 사이클이 이미 끝났나(그 경보로 도는 사이클은 막 복원된 좌석을 다시 비운다).
        /// 더 새 발화가 있다는 것만으로는 건너뛰지 않는다 — 느린 집행자가 매번 새 발화에 밀려 한 번도 집행하지 못한다.
        pub fn stale(&self, seq: u64) -> bool {
            seq <= self.resolved_through
        }

        fn strike(&mut self, at: f64, why: Why, drop: Option<(u8, u8)>, fire: (u64, f64), axis: Axis) -> Note {
            self.strikes = self.strikes.saturating_add(1);
            self.hold_until = self.hold_until.max(at + backoff(self.strikes));
            // ★(통합 minor 정리 · R1V42-3 · 발행 재료 — 판정 무관) 효과 없음 회차: 직전 효과 없음이 24시간 안에 없으면 1 부터. 효과 있는
            // 발화는 연속 수(strikes)를 0 으로 돌리지만 회차는 끊지 않는다 — 경보를 가끔 놓치는 집행자 좌석(둘 중 하나만 집행)에서
            // strike 1 feed 가 놓칠 때마다 새로 나가던 것(24시간 15~22건)을 회차의 2의 거듭제곱 번째(≤ 8건)로 묶는다.
            while self.ineff_log.front().is_some_and(|t| at - *t > 86_400.0) || self.ineff_log.len() > 1024 {
                self.ineff_log.pop_front();
            }
            if self.ineff_log.is_empty() {
                self.ineff_episode = 0;
            }
            self.ineff_episode = self.ineff_episode.saturating_add(1);
            self.ineff_log.push_back(at);
            Note::Ineffective {
                strikes: self.strikes,
                episode: self.ineff_episode,
                ineffective_24h: self.ineffective_within(at, 86_400.0),
                hold_until: self.hold_until,
                at,
                why,
                drop,
                fire_seq: fire.0,
                fire_at: fire.1,
                level: self.level[axis as usize].map(|(l, _)| l),
            }
        }

        /// 사이클 끝(표지 끔 — clear 실효 확인·모름 · 또는 Cycling 상한). `confirmed` = clear 실효 확인(feed 문구 전용).
        fn cycle_end(&mut self, at: f64, confirmed: bool) {
            if let Some(f) = self.fire.as_mut() {
                if !f.evaluated {
                    f.cleared = true;
                    f.tentative_until = None;
                }
            }
            self.resolved_through = self.seq;
            // 끝난 사이클은 집행자가 응답했다는 증거다 — 앞선 효과 없음(무응답·압축)의 보류는 여기서 끝난다(RNC4-1). 남겨 두면
            // 무응답이 쌓인 좌석이 clear 직후에도 '마지막 발화 + 백오프(k)'까지 막혀 자동 압축을 끈 좌석이 차단점을 넘는다(②).
            // strikes 는 다음 발화가 정한다(이 사이클 뒤 압축이면 +1 · 아니면 0) · 사이클 뒤 압축의 보류는 이 뒤에 걸린다. 다음
            // 발화는 여전히 최소 간격(600초)과 이 사이클 뒤 잰 수준 위 성장에 묶인다(I1·I3).
            self.hold_until = self.hold_until.min(at);
            self.scopes = Default::default();
            self.held = None;
            // ★(R3V3-1 ③) 자기보고 축 발화의 사이클이 끝났다 — 그 축의 사이클 뒤 첫 자기보고를 발화 값과 대조한다(report).
            self.sr_probe = self.fire.filter(|f| f.axis == Axis::SelfReport && !f.evaluated).and_then(|f| match f.notice {
                Verdict::Fire { pct, window, .. } => Some((f.seq, pct, window)),
                _ => None,
            });
            // (발행 전용) 24시간 사이클 기록.
            self.cycle_log.push_back((at, confirmed));
            while self.cycle_log.front().is_some_and(|(t, _)| at - *t > 86_400.0) || self.cycle_log.len() > 1024 {
                self.cycle_log.pop_front();
            }
            self.phase = Phase::Measuring {
                anchor: at,
                secs: CTX_GUARD_MEASURE_SECS,
                kind: Kind::AfterCycle,
                peak: [None; 2],
                hi: [None; 2],
                first: [None; 2],
                peak_scope: [0; 2],
                confirmed,
            };
        }

        /// 창을 닫는다 — 축별 R = 창 안 확인된 관측의 최고치 · 그 축의 다음 관측(최고치를 낸 범위 · 창이 비었으면 아무 범위)을
        /// R 에 접는다. `span` = 창이 잰 초 · `cut` = 낙폭(압축)이 끊었다 · `edge` = S 가장자리에서 닫은 보고(부른 쪽이 그 축의
        /// 접기를 풀고 그 보고로 곧바로 판정한다).
        #[allow(clippy::too_many_arguments)]
        fn close(
            &mut self,
            anchor: f64,
            kind: Kind,
            peak: [Option<Lv>; 2],
            first: [Option<Lv>; 2],
            peak_scope: [u64; 2],
            confirmed: bool,
            span: f64,
            cut: bool,
            edge: Option<Lv>,
        ) -> Note {
            let mut base = [None; 2];
            for a in AXES {
                let i = a as usize;
                if peak[i].is_some() {
                    base[i] = self.scopes[i].iter().find(|e| scope_id(&e.0) == peak_scope[i]).map(|e| (e.3).2);
                }
                self.level[i] = peak[i].map(|l| (l, kind));
                self.fold[i] = Some(if peak[i].is_some() { peak_scope[i] } else { 0 });
                // 새 창이 수준을 다시 쟀다 — 앞 창의 가장자리 표시는 끝났다(가장자리 닫힘이면 부른 쪽이 그 축을 다시 세운다).
                self.edge_mark[i] = None;
                self.edge_seen[i] = None;
            }
            self.phase = Phase::Free;
            Note::Measured { kind, level: peak, first, base, confirmed, span, cut, edge, at: anchor + span }
        }

        /// 시한 처리 — 보고 없이도 부른다(수집기 틱). 한 번에 여러 시한이 지났으면 차례로 모두 처리한다.
        fn expire(&mut self, now: f64, frozen: bool, notes: &mut Vec<Note>) {
            for _ in 0..4 {
                match self.phase {
                    Phase::Awaiting { at, deadline } => {
                        if frozen {
                            // 배달 동결 중에는 사이클이 올 수 없다 — 시한을 멈춘다(재개 뒤 온전한 시한).
                            self.phase = Phase::Awaiting { at, deadline: deadline.max(now + CTX_GUARD_CLEAR_WAIT_SECS) };
                            return;
                        }
                        if now < deadline {
                            return;
                        }
                        self.phase = Phase::Free;
                        let k1 = self.strikes.saturating_add(1);
                        let retry = deadline + backoff(k1);
                        if let Some(f) = self.fire.as_mut() {
                            if !f.cleared && !f.evaluated {
                                f.tentative_until = Some(retry);
                                notes.push(Note::Unanswered { seq: f.seq, retry_after: retry });
                                // 보류 만료에 발화 축(실측)의 **가장 최근 관측**을 다시 판정한다(보고가 끊긴 좌석도 재발화 — I2 ·
                                // 표지 없는 clear 뒤 새 세션이 막대 아래면 발화하지 않는다 — RR2-ROLE-1). 자기보고 축은 다음
                                // status.set 도착 때 판정한다.
                                if f.axis == Axis::Measured {
                                    self.held = Some(Held { until: retry, axis: f.axis });
                                }
                            }
                        }
                    }
                    Phase::Cycling { since } => {
                        if now < since + CTX_GUARD_CYCLING_MAX_SECS {
                            return;
                        }
                        // 끔이 끝내 오지 않았다 — 결과를 모르는 사이클 끝(종전과 같다).
                        self.cycle_end(since + CTX_GUARD_CYCLING_MAX_SECS, false);
                    }
                    Phase::Measuring { anchor, secs, kind, peak, first, peak_scope, confirmed, .. } => {
                        if now < anchor + secs {
                            return;
                        }
                        notes.push(self.close(anchor, kind, peak, first, peak_scope, confirmed, secs, false, None));
                    }
                    Phase::Free => return,
                }
            }
        }

        /// (검체 전용 줄임) 사이클 표지 — `on` = 켬 · `!on` = 결과 모름 끔(구 CLI·데몬 해제와 같다). 운영 경로는 [`ClearGuard::cycle_on`]
        /// · [`ClearGuard::cycle_off`] 를 직접 부른다(`surface.quiesce` · `governance::release_quiescing`).
        #[cfg(test)]
        pub fn cycle(&mut self, on: bool, now: f64, frozen: bool) -> Out {
            if on {
                self.cycle_on(now, frozen)
            } else {
                self.cycle_off(Outcome::Unknown, now, frozen)
            }
        }

        /// 표지 켬(clear 직전). 보류 기록은 둔다 — 사이클이 clear 없이 끝나면(NotCleared) 틱이 그 관측을 다시 판정해야 한다
        /// (끝난 사이클은 `cycle_end` 가 지운다).
        pub fn cycle_on(&mut self, now: f64, frozen: bool) -> Out {
            let mut notes = vec![];
            self.expire(now, frozen, &mut notes);
            if !matches!(self.phase, Phase::Cycling { .. }) {
                self.before_cycle = self.phase;
                self.phase = Phase::Cycling { since: now };
            }
            Out { notes, verdict: None }
        }

        /// 표지 끔 + 결과. 확인·모름 = 사이클 끝(발화 풀림 · 보류 끝 · 수준 재측정). **clear 안 됨 = 사이클 끝이 아니다** — 표지 켬
        /// 전 상태로 돌아간다(Awaiting 시한·재는 창·수준·resolved_through·보류 무변경 · 그 발화는 미해결 → 같은 --fire 재집행 가능 ·
        /// 시한이 지나면 잠정 보류 뒤 효과 없음으로 재발화). 표지 켬 없이 온 끔은 무시한다(Cycling 상한이 이미 끝냈다).
        pub fn cycle_off(&mut self, outcome: Outcome, now: f64, frozen: bool) -> Out {
            let mut notes = vec![];
            self.expire(now, frozen, &mut notes);
            if matches!(self.phase, Phase::Cycling { .. }) {
                if outcome == Outcome::NotCleared {
                    self.phase = std::mem::take(&mut self.before_cycle);
                    self.expire(now, frozen, &mut notes);
                } else {
                    self.cycle_end(now, outcome == Outcome::Cleared);
                }
            }
            Out { notes, verdict: None }
        }

        /// 수집기 틱 — 시한 처리 · Free 이고 보류가 끝났으면 그 축의 **가장 최근 관측**을 다시 판정한다(가드 밖 값을 읽지 않는다
        /// · 보류해 둔 값을 되살리지 않는다 — 그 사이 다른 범위(표지 없는 clear 뒤 새 세션 · 헬퍼)의 보고가 왔으면 그 값이다).
        pub fn tick(&mut self, now: f64, frozen: bool) -> Out {
            let mut notes = vec![];
            self.expire(now, frozen, &mut notes);
            let mut verdict = None;
            if self.phase == Phase::Free {
                if let Some(h) = self.held {
                    if now >= h.until {
                        self.held = None;
                        if let Some(o) = self.last_obs[h.axis as usize] {
                            verdict = Some(self.decide(o.pct, o.window, h.axis, o.base, o.scope, o.at, now, &mut notes));
                        }
                    }
                }
            }
            Out { notes, verdict }
        }

        /// 보류 재판정이 판정할 관측(그 축의 가장 최근 관측) — (범위 지문, 퍼센트). 검체·성질 오라클 전용 조회.
        #[cfg(test)]
        pub fn retry_obs(&self, axis: Axis) -> Option<(u64, u8)> {
            self.last_obs[axis as usize].map(|o| (o.scope, o.pct))
        }

        /// 재배달 — 집행 시도가 사이클을 끝내지 못하고 물러났다(단일 비행 점유 해제 · 점유자 사망). 그 발화가 아직 미해결(Awaiting ·
        /// 사이클 표지 전 또는 clear 안 됨 끔 뒤)이면 **같은 통보를 한 번** 돌려준다(같은 번호 · 발화 아님 — last_fire·strikes·seq
        /// 무변경 · I1 무관). rc 88(진행 중)로 물러난 집행자가 턴 안에서 기다리지 않아도 점유자가 clear 전에 실패한 통보를 다시
        /// 받는다(RR2-ROLE-2). 발화 1건당 최대 1회(실패가 되풀이되는 좌석의 저장 지시 고리 없음 — 그 뒤는 시한·잠정 보류).
        pub fn redeliver(&mut self, now: f64, frozen: bool) -> Out {
            let mut notes = vec![];
            self.expire(now, frozen, &mut notes);
            let mut verdict = None;
            if matches!(self.phase, Phase::Awaiting { .. }) {
                if let Some(f) = self.fire.as_mut() {
                    if !f.cleared && !f.evaluated && !f.redelivered {
                        f.redelivered = true;
                        verdict = Some(f.notice);
                    }
                }
            }
            Out { notes, verdict }
        }

        /// 낙폭 판정(같은 축 · 같은 범위). 참 = 압축(자동·수동 /compact · 수동 /clear 를 구분하지 못한다). (전, 후) 를 돌려준다.
        fn scope_drop(&mut self, axis: Axis, scope: &str, lv: Lv, base: u8) -> Option<(u8, u8)> {
            let sc = &mut self.scopes[axis as usize];
            let last = (lv.0, lv.1, base);
            match sc.iter().position(|e| e.0 == scope) {
                Some(i) => {
                    let (f, pk, confirmed, _) = sc.remove(i);
                    let before = rebase(pk, lv.1);
                    let low = u16::from(lv.0) + u16::from(CTX_GUARD_COMPACT_DROP) <= u16::from(before);
                    let dropped = low && confirmed;
                    // 확인 전 첫 관측보다 크게 낮으면 그 첫 관측을 버린다(낡은 값) · 아니면 확인된다.
                    sc.insert(0, (f, if low { lv } else { higher(pk, lv) }, true, last));
                    dropped.then_some((before, lv.0))
                }
                None => {
                    sc.insert(0, (scope.to_string(), lv, false, last));
                    sc.truncate(CTX_GUARD_SCOPES_KEPT);
                    None
                }
            }
        }

        /// 관측 1건 — 낙폭·창·수준을 처리하고 Free 면 발화를 판정한다.
        pub fn report(&mut self, r: &Rep) -> Out {
            let mut notes = vec![];
            self.expire(r.now, r.frozen, &mut notes);
            let i = r.axis as usize;
            let lv = (r.pct, r.window);
            let sid = scope_id(r.scope);
            if matches!(self.phase, Phase::Cycling { .. }) {
                // clear·복원 도중 — 이 관측은 수준도 낙폭도 아니다(clear 의 3% · 재주입 중간값 · 새 세션 첫 보고).
                return Out { notes, verdict: Some(Verdict::Quiet) };
            }
            // ★(수정 4회차 · R3V3-1 ③ · 수정 5회차 V41NC-1) 고착 자기보고 — 자기보고 축 발화 뒤 끝난 사이클의 **그 축 첫 보고**가 발화 때와
            // 정확히 같은 값이고 그 값이 **차단점 이상으로만 보이는 값**(B = `blocked_floor` · 200K·미상 89 · 1M 99)이면 그 사이클은 이 축에서
            // 효과 증거가 없다: 그 발화의 결과를 효과 없음 1회로 정한다(strike · 지수 백오프 ≤ 7200 · I2). B 아래는 쓰지 않는다 — 차단점
            // 전의 좌석이 보일 수 있는 값이라 정직한 재성장과 관측이 같다: S 아래는 막대가 그 위라(G 성장) 고리가 없고(설계 단위 핀 F3),
            // S~C(가장자리 띠와 C — C 는 차단점 아래로 보이는 가장 큰 값)는 창보다 드문 정직한 보고가 복원 뒤 재성장으로 발화 값과 같은
            // 첫 보고를 내는 좌석(주기 좌석은 매 사이클)과 고착 보고가 같은 관측이다 — 그 보고를 고착으로 세면 보류 동안 차단점을 넘은
            // 정직한 좌석은 보고조차 못 해(자기보고 축은 보류 만료에 재판정하지 않는다) 영구 무clear 다(060075e8 의 C 이상 범위 · V41NC-1 ·
            // 모형 44/4,440행 · 드릴 v41-srh 거부 59/54). 그래서 ② 우선(예외 ⓑ·ⓒ 의 최소 간격 고리 · 오너 결재 1). 값이 바뀌면 정상 규칙이다
            // (단 이미 걸린 보류는 그 값과 무관하게 hold_until 까지다). 실측 축은 쓰지 않는다(관측 동치 · ②).
            // 이 보고는 아래에서 여느 관측처럼 수준·판정을 거친다(판정은 보류가 막는다).
            if r.axis == Axis::SelfReport {
                if let Some((seq, p, w)) = self.sr_probe.take() {
                    if let Some(f) = self.fire.filter(|f| f.seq == seq && f.cleared && !f.evaluated) {
                        if rebase((p, w), r.window) == r.pct && r.pct >= blocked_floor(r.window) {
                            if let Some(fm) = self.fire.as_mut() {
                                fm.evaluated = true;
                                fm.tentative_until = None;
                            }
                            let n = self.strike(r.now, Why::SelfReportUnchanged, Some((p, r.pct)), (f.seq, f.at), Axis::SelfReport);
                            notes.push(n);
                        }
                    }
                }
            }
            let s = stop_cap(r.window);
            // S 이상 외톨이 보고(사이클 뒤 창에서 이 범위의 S 미만 보고도 앞선 S 이상 보고도 없다) — 확인 전에는 수준도 낙폭
            // 기준도 아니다. 다른 범위(헬퍼·다른 파일)의 보고는 이 확인에 쓰지 않는다.
            if let Phase::Measuring { kind: Kind::AfterCycle, ref peak, ref peak_scope, ref mut hi, .. } = self.phase {
                let same_peak = peak[i].is_some() && peak_scope[i] == sid;
                let same_hi = hi[i].is_some_and(|(_, h)| h == sid);
                if r.pct >= s && !same_peak && !same_hi {
                    hi[i] = Some((lv, sid));
                    return Out { notes, verdict: Some(Verdict::Quiet) };
                }
            }
            let dropped = self.scope_drop(r.axis, r.scope, lv, r.base);
            // 보류 재판정의 재료 — 이 축의 가장 최근 관측(범위 무관 · RR2-ROLE-1).
            self.last_obs[i] = Some(LastObs { scope: sid, pct: r.pct, window: r.window, base: r.base, at: r.now });
            if let Some(drop) = dropped {
                // 압축(같은 범위 10%p 낙폭). 진행 중인 재는 창은 낙폭 **전**까지의 최고치로 먼저 닫는다(RR1-ROLE-2) — 그 창이 잰
                // 사실(사이클 뒤 수준)이 버려지면 효과 없음 feed 가 앞 창의 값·'관측 없음'을 싣고 사이클 뒤 수준 보고가 끝내 나가지
                // 않는다. 수준은 곧이어 압축 뒤 창이 다시 잰다(판정 무변경 — 창이 열린 동안은 발화하지 않는다).
                if let Phase::Measuring { anchor, kind, peak, first, peak_scope, confirmed, .. } = self.phase {
                    notes.push(self.close(anchor, kind, peak, first, peak_scope, confirmed, (r.now - anchor).max(0.0), true, None));
                }
                // 발화 결과를 한 번만 정한다.
                if let Some(f) = self.fire {
                    if !f.evaluated {
                        if let Some(fm) = self.fire.as_mut() {
                            fm.evaluated = true;
                            fm.tentative_until = None;
                        }
                        if f.cleared {
                            // 사이클이 끝났는데 다음 발화 전에 압축 — 그 clear 는 압축을 막지 못했다.
                            let n = self.strike(r.now, Why::DropAfterCycle, Some(drop), (f.seq, f.at), r.axis);
                            notes.push(n);
                        } else {
                            // 사이클 전에 압축이 왔다(Claude 가 먼저 · 또는 오너 /compact) — 효과 없음이 아니다(결과 확정 · strike 없음).
                            notes.push(Note::DropBeforeCycle { seq: f.seq, drop });
                        }
                    }
                }
                self.held = None;
                let mut peak = [None; 2];
                peak[i] = Some(lv);
                let mut first = [None; 2];
                first[i] = Some(lv);
                let mut peak_scope = [0; 2];
                peak_scope[i] = sid;
                self.phase = Phase::Measuring {
                    anchor: r.now,
                    secs: CTX_GUARD_REREAD_SECS,
                    kind: Kind::AfterCompaction,
                    peak,
                    hi: [None; 2],
                    first,
                    peak_scope,
                    confirmed: false,
                };
                return Out { notes, verdict: Some(Verdict::Quiet) };
            }
            if let Phase::Measuring { anchor, secs, kind, mut peak, mut hi, mut first, mut peak_scope, confirmed } = self.phase {
                if first[i].is_none() {
                    first[i] = Some(lv);
                }
                let early = kind == Kind::AfterCycle && r.pct >= s;
                if early {
                    // S 가장자리(자동 압축이 돌지 않는 좌석) — 확인된 이 보고로 창을 곧바로 닫고 **이 보고로 곧바로 판정한다**
                    // (아래 Free 판정 · 막대 ≤ S). 확인 = 창 최고치를 낸 범위가 이 보고의 범위다 · 또는 같은 범위의 두 번째 S 이상
                    // 보고(외톨이 짝). 이 보고는 수준에 싣지 않고 그 축은 접지 않는다(RNC5-1): 자동 압축을 끈 200K 좌석의 S→차단점
                    // 여유(3.5%p)는 작업 한 덩어리보다 작을 수 있어, 이 보고를 수준에 접고 R+1 을 기다리면 다음 덩어리가 차단점을 한
                    // 번에 넘는다(a3832314 — 저장 지시 거부 · 끝까지 막힘 · ②). ★(수정 6회차 · V42NC-1) **외톨이 짝도 같다** — 짝의 낮은
                    // 값도 수준에 넣지 않는다(창 안 S 미만 관측이 없으면 수준 미상 → 막대 = 기본). 종전(e09a6af1 ~ 588fb592)은 짝의 낮은
                    // 값을 R 에 넣어 막대가 R+1(200K 착지 86 → 87)로 올랐고, 확인된 가장자리 보고가 통보되지 않은 채 덩어리 하나로
                    // 차단점을 넘어 저장 지시가 거부됐다(자기보고 축은 복원 뒤에야 보고하므로 창 안 첫 두 보고가 착지 값의 짝이 되는 것이
                    // 자연 경로 · 드릴 v42-pair-head-60 끝까지 2,883초 · 거부 27 — 보고조차 못 해 영구 무clear). 대가는 설계 I3 예외 ⓑ
                    // 다 — 사이클 뒤 10분 안에 S 로 돌아오는 좌석은 복원만으로도 사이클마다 한 번 발화한다(작업으로 돌아온 좌석과 같은
                    // 관측값 열 · 최소 간격에 묶임 · 오너 결재 ① · feed 처방).
                    notes.push(self.close(anchor, kind, peak, first, peak_scope, confirmed, (r.now - anchor).max(0.0), false, Some(lv)));
                    self.fold[i] = None;
                    // 이 축의 다음 발화는 가장자리 복귀 발화다(R3V3-1 ① — 연속 수·오너 feed · 판정 무관) — 이 보고의 범위로(R1V42-1).
                    self.edge_mark[i] = Some(sid);
                    self.edge_seen[i] = Some(sid);
                } else {
                    if hi[i].is_some_and(|(_, h)| h == sid) {
                        hi[i] = None; // 같은 범위의 S 아래 보고가 오면 외톨이 S 보고는 버린다(낡은 첫 보고)
                    }
                    if peak[i].is_none_or(|pk| rebase(pk, lv.1) < lv.0) {
                        peak_scope[i] = sid;
                    }
                    peak[i] = Some(peak[i].map_or(lv, |pk| higher(pk, lv)));
                    self.phase = Phase::Measuring { anchor, secs, kind, peak, hi, first, peak_scope, confirmed };
                    return Out { notes, verdict: Some(Verdict::Quiet) };
                }
            }
            if matches!(self.phase, Phase::Awaiting { .. }) {
                return Out { notes, verdict: Some(Verdict::Quiet) };
            }
            // Free
            if self.fold[i].is_some_and(|f| f == 0 || f == sid) {
                self.fold[i] = None;
                // ★(수정 4회차 · RNC6-1) 접기는 S 미만 관측에만 — S 이상 관측은 접기 기회만 소비하고 접기 전 막대로 곧바로 판정한다
                // (아래 decide · 창 안 S 가장자리와 같은 규칙 · 그 관측은 수준 밖). 자동 압축을 끈 200K 좌석은 덩어리 간격이 창보다
                // 길고 사이 보고가 없으면 창 뒤 첫 관측이 S 를 넘는 덩어리다 — 그것을 접어 R+1 을 기다리면 다음 덩어리가 차단점(88.5)을
                // 한 번에 넘고 저장 지시가 거부된다(e09a6af1~02bc0079 · 영구 무clear ②). 대가: 설계 I3 예외 ⓑ 가 창 뒤 첫 관측까지
                // 넓어진다(최소 간격에 묶임).
                if r.pct < s {
                    let (l, kind) = match self.level[i] {
                        Some((l, k)) => (higher(l, lv), k),
                        None => (lv, Kind::AfterCycle),
                    };
                    self.level[i] = Some((l, kind));
                    notes.push(Note::MeasuredLate { axis: r.axis, level: l });
                    return Out { notes, verdict: Some(Verdict::Quiet) };
                }
                // 창 뒤 첫 관측이 가장자리 이상 — 예외 ⓑ(창 뒤)의 발화다: 복귀 계수에 넣는다(발행 재료 · 판정 무관 — 창보다 드물게
                // 보고하는 좌석의 가장자리 고리도 오너 feed 에 보인다) — 이 관측의 범위로(R1V42-1).
                self.edge_mark[i] = Some(sid);
                self.edge_seen[i] = Some(sid);
            }
            let v = self.decide(r.pct, r.window, r.axis, r.base, sid, r.now, r.now, &mut notes);
            Out { notes, verdict: Some(v) }
        }

        /// 발화 판정(Free). `observed_at` = 그 관측이 가드에 온 시각(발행 재료 — 보고 판정이면 지금 · 보류 재판정이면 그 축의
        /// 가장 최근 관측 시각).
        #[allow(clippy::too_many_arguments)]
        fn decide(
            &mut self,
            pct: u8,
            window: Option<u64>,
            axis: Axis,
            base: u8,
            scope: u64,
            observed_at: f64,
            now: f64,
            notes: &mut Vec<Note>,
        ) -> Verdict {
            debug_assert!(self.phase == Phase::Free);
            let bar = self.bar(axis, base, window);
            if pct < bar {
                // 보류는 그대로 둔다 — 만료 때 그 축의 가장 최근 관측(바로 이 막대 아래 값 · 또는 그 뒤 관측)을 다시 판정한다.
                // ★(수정 5회차 · V41R-2 · 발행 전용) 막대 아래 판정은 그 축의 가장자리 표시를 끝낸다 — 표시는 '이 축의 다음 판정이 가장자리
                // 보고의 판정'이라는 뜻이고, 그 판정이 무발화로 끝났으면(표지 없는 clear 뒤 새 세션 · 압축된 좌석) 뒤이은 성장 발화는 복귀
                // 발화가 아니다(종전: 표시가 남아 몇 시간 뒤 성장 발화가 '가장자리 복귀' error feed · 처방으로 나갔다 — 오진). 판정 무관.
                // ★(통합 minor 정리 · R1V42-1) **같은 범위**의 막대 아래 판정만 지운다 — 다른 범위(헬퍼·claude -p 의 낮은 상태줄 · 다른
                // 세션 파일)의 관측 한 건이 본체의 가장자리 표시를 지우면 계속 도는 고리가 복귀 계수·error feed·좌석 행에서 사라진다(오너
                // 결재 ① 의 가시성 조건). 표지 없는 clear 뒤 새 세션은 다른 범위라 그 성장 발화는 복귀가 아니다(아래 발화 판정).
                if self.edge_mark[axis as usize] == Some(scope) {
                    self.edge_mark[axis as usize] = None;
                }
                return Verdict::Quiet;
            }
            let tentative =
                self.fire.filter(|f| !f.cleared && !f.evaluated).and_then(|f| f.tentative_until).unwrap_or(f64::NEG_INFINITY);
            let spacing = self.last_fire.map_or(f64::NEG_INFINITY, |f| f + CTX_GUARD_MIN_SPACING_SECS);
            let until = self.hold_until.max(tentative).max(spacing);
            if now < until {
                // 틱의 재판정은 실측 축만(자기보고는 다음 status.set 도착 때 다시 판정한다 — 낡은 자기보고를 되살리지 않는다).
                if axis == Axis::Measured {
                    self.held = Some(Held { until, axis });
                }
                return Verdict::Held { bar, until };
            }
            // 직전 발화의 결과(아직 정해지지 않았으면): 사이클을 봤으면 효과 있음(연속 효과 없음 0) · 못 봤으면 효과 없음 +1.
            if let Some(f) = self.fire {
                if !f.evaluated {
                    if f.cleared {
                        self.strikes = 0;
                    } else {
                        let n = self.strike(now, Why::NoCycle, None, (f.seq, f.at), axis);
                        notes.push(n);
                    }
                }
            }
            self.seq += 1;
            let lvl = self.level[axis as usize];
            // ★(수정 4회차 · R3V3-1 ①②) 가장자리·C 띠 복귀 발화의 연속 수(발행 재료 — 판정 무관): 직전 발화 뒤 cys 사이클이 끝났고
            // (복귀 = 사이클 뒤) 이 발화가 사이클 뒤 창을 S 가장자리에서 닫았거나 창 뒤 첫 관측이 S 이상이었던 축의 발화이거나 잰 수준이
            // C 이상(막대 = C)이면 복귀 발화다.
            // 사이클 뒤 성장으로 난 발화는 연속을 끊는다 · 사이클 없이 난 발화(부트 · 무응답 재통보)는 연속을 바꾸지 않는다.
            let level_now = lvl.map(|(l, _)| rebase(l, window));
            let c_band = level_now.is_some_and(|l| l >= block_cap(window));
            let after_cycle = self.fire.is_some_and(|f| f.cleared);
            // ★(통합 minor 정리 · R1V42-1 · 발행 재료 — 판정 무관) 복귀 = 그 축의 가장자리 표시가 **이 관측의 범위**다 · C 띠 · 또는 이
            // 사이클 뒤 **같은 범위가 가장자리(예외 ⓑ)에 닿았고** 잰 수준 R 이 있고 통보 퍼센트가 R + G 미만이다(자기보고 축처럼 범위가 하나인
            // 좌석이 가장자리에 닿은 뒤 ±1~2 흔들림으로 표시를 지워도 셈이 남는다).
            // ★(게이트 수정 1회차 ROLE-G2) 'R + G 미만'만으로는 세지 않는다 — 사이클 뒤 수준 81~84(200K · 설계가 정상이라 적은 68~84 대역)
            // 에서는 R + 5 가 S(85)를 넘어 막대가 max(S, R+1) = 85 로 묶이므로, 가장자리 관측 없이 작업으로 1~4%p 자라 난 **성장 발화**도
            // 통보 퍼센트 < R + G 다. 종전(ddad22d9)은 그 발화를 가장자리 복귀로 세어 오너 **error** feed(회차 1)를 내고 본문이 일어나지 않은
            // 원인('창 안 가장자리 도달 · 창 뒤 첫 보고가 가장자리 이상 · 또는 C 이상')을 단정했다. 이제 그 사이클 뒤 같은 범위의 가장자리
            // 관측(`edge_seen`)이 있어야 수치 기준이 든다 — 복귀로 세는 모든 발화는 feed 본문의 세 원인 중 하나를 실제로 거쳤다.
            let below_growth = self.edge_seen[axis as usize] == Some(scope)
                && level_now.is_some_and(|l| u16::from(pct) < u16::from(l) + u16::from(CTX_GUARD_GROWTH));
            let edge_fire = after_cycle && (self.edge_mark[axis as usize] == Some(scope) || c_band || below_growth);
            self.edge_mark[axis as usize] = None;
            self.edge_seen[axis as usize] = None;
            self.sr_probe = None;
            if edge_fire {
                self.edge_run = self.edge_run.saturating_add(1);
                // ★(수정 5회차 · R1V4-1) 고리 회차 — 직전 복귀 발화가 24시간 안에 없으면 1 부터 다시 센다(시각만으로 끝난다 · 성장 발화는
                // 회차를 끊지 않는다). 오너 error feed 는 회차의 2의 거듭제곱 번째에만 난다: 회차가 새로 시작하려면 24시간 공백이
                // 있어야 하므로 어느 24시간 창도 한 회차의 연속 구간만 닿고, I1(최소 간격 600초)이 그 안의 복귀 발화를 ≤ 144 로 묶어
                // 거듭제곱은 ≤ 8(1·2·4·…·128)이다 — 번갈이·속도 변동 좌석에서도 스톰이 없다(종전 연속 수는 성장 발화마다 0 으로 돌아가
                // 가장자리 발화마다 '1번째'였다).
                while self.edge_log.front().is_some_and(|t| now - *t > 86_400.0) || self.edge_log.len() > 1024 {
                    self.edge_log.pop_front();
                }
                if self.edge_log.is_empty() {
                    self.edge_episode = 0;
                }
                self.edge_episode = self.edge_episode.saturating_add(1);
                self.edge_log.push_back(now);
                let (cycles_24h, confirmed_24h) = self.cycles_within(now, 86_400.0);
                notes.push(Note::EdgeReturn {
                    run: self.edge_run,
                    episode: self.edge_episode,
                    returns_24h: self.edge_returns_within(now, 86_400.0),
                    seq: self.seq,
                    pct,
                    bar,
                    level: level_now,
                    axis,
                    window,
                    c_band,
                    cycles_24h,
                    confirmed_24h,
                    confirmed: self.cycle_log.back().is_some_and(|(_, c)| *c),
                });
            } else if after_cycle {
                self.edge_run = 0;
            }
            let notice = Verdict::Fire {
                pct,
                bar,
                level: lvl.map(|(l, _)| rebase(l, window)),
                strikes: self.strikes,
                seq: self.seq,
                after_compaction: lvl.is_some_and(|(_, k)| k == Kind::AfterCompaction),
                axis,
                base,
                window,
                observed_age: (now - observed_at).max(0.0),
            };
            self.fire = Some(FireRec {
                seq: self.seq,
                at: now,
                axis,
                cleared: false,
                evaluated: false,
                tentative_until: None,
                scope,
                notice,
                redelivered: false,
            });
            self.last_fire = Some(now);
            self.held = None;
            self.phase = Phase::Awaiting { at: now, deadline: now + CTX_GUARD_CLEAR_WAIT_SECS };
            notice
        }
    }
}

/// 낙폭 판정 범위 = 관측의 세션 파일 **줄기**(세션 id) — 심링크·`/private` 표기 차이는 같은 범위다. 빈 경로는 "" 범위(자기보고와
/// 이름이 같아도 축이 달라 섞이지 않는다). 범위는 수준·막대·발화에 쓰이지 않는다(세션 키는 가드 입력이 아니다).
pub(crate) fn ctx_guard_scope(session_file: &str) -> String {
    let t = session_file.trim();
    if t.is_empty() {
        return String::new();
    }
    std::path::Path::new(t).file_stem().map_or_else(|| t.to_string(), |s| s.to_string_lossy().into_owned())
}

/// 데몬 단조 초(`started_instant` 기준) — 가드의 시각.
pub(crate) fn ctx_guard_now(daemon: &Daemon) -> f64 {
    daemon.started_instant.elapsed().as_secs_f64()
}

/// ★(게이트 수정 1회차 R1R3-1 · 순수) 단조 시각 `at_mono`(가드 발화 시각)의 epoch 표시값 = 지금 epoch − (지금 단조 − `at_mono`). 데몬 기동
/// 시각(`started_at`)을 더하지 않는다 — 기동 뒤 누적 절전만큼 벽시계가 단조 시계보다 앞서 있어 그 합은 과거로 밀린다.
pub(crate) fn awaiting_since_epoch(wall_now: f64, mono_now: f64, at_mono: f64) -> f64 {
    wall_now - (mono_now - at_mono).max(0.0)
}

/// ★(clear 가드 v3) 수집기 틱 — 시한 처리와 보류해 둔 실측 관측의 재판정 하나뿐이다. **가드 밖 값을 읽지 않는다**
/// (observed_usage·agent_status·오버라이드 파일 무접촉 · 입력은 단조 시각과 배달 동결뿐). 발행은 가드 락을 놓은 뒤.
pub(crate) fn ctx_guard_tick(daemon: &Daemon, s: &Surface) {
    ctx_guard_tick_at(daemon, s, ctx_guard_now(daemon));
}

/// [`ctx_guard_tick`] 의 본체 — 단조 시각을 받는다(검체가 시한·보류를 실제로 기다리지 않게 · 운영 경로는 언제나 위의 현재 값).
/// 점유자가 죽어 풀지 못한 단일 비행 점유(Drop 없이 끝난 cycle-agent)도 여기서 버린다 — 버렸으면 그 시도는 사이클을 끝내지 못한
/// 것이므로 미해결 발화를 한 번 재배달한다(RR2-ROLE-2 · [`ctx_guard_claim_ended`]).
pub(crate) fn ctx_guard_tick_at(daemon: &Daemon, s: &Surface, now: f64) {
    let frozen = daemon.paused.load(Ordering::Relaxed);
    let out = s.ctx_loop_guard.lock().unwrap_or_else(|e| e.into_inner()).tick(now, frozen);
    if out.verdict.is_some() || !out.notes.is_empty() {
        crate::handlers::publish_ctx_guard(daemon, s, out, "held-retry", None);
    }
    // 점유 락은 가드 락을 놓은 뒤 따로(가드 락은 말단 — 중첩 없음). 점유 나이는 데몬 **단조** 초의 현재 값으로 잰다(점유가 그 시계로
    // 찍혔다 · 게이트 수정 1회차 R1R3-1 — 벽시계는 절전만큼 뛰어 산 집행자의 점유를 버렸다).
    let swept = {
        let mut c = s.cycle_claim.lock().unwrap_or_else(|e| e.into_inner());
        let dead = c.as_ref().is_some_and(|c| c.expired(ctx_guard_now(daemon)));
        if dead {
            *c = None;
        }
        dead
    };
    if swept {
        ctx_guard_claim_ended(daemon, s);
    }
}

/// ★(RR2-ROLE-2) 단일 비행 점유가 끝났다(점유자가 풀었다 · 죽었다) — 그 시도가 사이클을 끝내지 못했으면(가드가 아직 Awaiting ·
/// 표지 켬 전 실패 또는 clear 안 됨 끔 뒤) 같은 통보를 한 번 재배달한다(`context.threshold` · 같은 fire_id · `redelivery`). rc 88 로
/// 물러난 집행자는 턴 안에서 기다리지 않는다 — 이 재배달이 다음 판정의 계기다. 발행은 가드 락 밖.
pub(crate) fn ctx_guard_claim_ended(daemon: &Daemon, s: &Surface) {
    let now = ctx_guard_now(daemon);
    let frozen = daemon.paused.load(Ordering::Relaxed);
    let out = s.ctx_loop_guard.lock().unwrap_or_else(|e| e.into_inner()).redeliver(now, frozen);
    if out.verdict.is_some() || !out.notes.is_empty() {
        crate::handlers::publish_ctx_guard(daemon, s, out, "redelivery", None);
    }
}

/// ★(clear 가드 v3) 좌석 상태 JSON(`org.status` = `cys status --json` 행)의 `ctx_guard` — autopilot 게이트 3(미해결 발화)의
/// 입력. `phase`(free|awaiting|cycling|measuring) · `fire_id`(마지막 발화) · `awaiting_since`(epoch · Awaiting 일 때) ·
/// `level_pct`(실측 축 잰 수준) · `self_report_level_pct` · `strikes` · `resolved_through` · `claim`(★RR2-ROLE-2 — 진행 중인 사이클
/// 단일 비행 점유 `{holder_pid, since(epoch), fire_id}` · 없거나 죽은 점유면 null — 0단계에서 잡혀 quiescing(5단계)보다 먼저 보인다)
/// · ★(수정 4회차 · R3V3-1 ②) `edge_run`(연속 가장자리·C 띠 복귀 발화 수 — 성장으로 난 발화가 0 으로) · `clears_24h`(24시간 끝난
/// cys 사이클 수) · `confirmed_clears_24h`(그중 clear 실효 확인). 막대는 싣지 않는다 — 기본 임계가 역할 오버라이드 파일이라 상태 조회마다 좌석 수만큼 파일을 열게 된다(막대는
/// `context.threshold`·`context.level_measured` 에 있다).
pub(crate) fn ctx_guard_wire(daemon: &Daemon, s: &Surface) -> Value {
    use clear_guard::Phase;
    // ★(수정 6회차 V42R-1) 비동기 사이클 작업(`cys cycle-agent --detach`) — 작업 락은 가드 락 전에 따로(중첩 없음).
    let job = crate::cycle_jobs::seat_job(daemon, s.id);
    // 점유 락과 가드 락은 따로 잡는다(중첩 없음).
    let claim = {
        let c = s.cycle_claim.lock().unwrap_or_else(|e| e.into_inner());
        c.as_ref()
            .filter(|c| !c.expired(ctx_guard_now(daemon)))
            .map(|c| json!({"holder_pid": c.pid, "since": c.since, "fire_id": c.fire_id}))
    };
    let g = s.ctx_loop_guard.lock().unwrap_or_else(|e| e.into_inner());
    let now = ctx_guard_now(daemon);
    // ★(게이트 수정 1회차 R1R3-1) 발화 시각(단조 `at`)의 epoch 는 **지금에서 단조 경과를 뺀 값**이다. 종전 `started_at + at` 은 데몬 기동
    // 뒤 누적 절전만큼 과거로 밀려(macOS 단조 시계는 절전 중 멈춘다) 5분 보고가 방금 난 통보를 'clear 통보 미집행 N분+' 로 찍었다.
    // 경과는 가드 시한(1200초)·cycle-agent 예산과 같은 단조 경과다(통보 뒤 절전은 미집행 시간에 넣지 않는다 — 그동안 집행자도 잤다).
    let awaiting_since = match g.phase {
        Phase::Awaiting { at, .. } => Some(awaiting_since_epoch(crate::state::now_epoch(), now, at)),
        _ => None,
    };
    let (clears_24h, confirmed_24h) = g.cycles_within(now, 86_400.0);
    json!({
        "phase": g.phase.as_str(),
        "fire_id": g.fire.map(|f| crate::handlers::ctx_guard_fire_id(daemon, s.id, f.seq)),
        "awaiting_since": awaiting_since,
        "level_pct": g.level[0].map(|((p, _), _)| p),
        "self_report_level_pct": g.level[1].map(|((p, _), _)| p),
        "strikes": g.strikes,
        "resolved_through": g.resolved_through,
        "claim": claim,
        "job": job,
        "edge_run": g.edge_run,
        // ★(수정 5회차 · R1V4-1) 고리 회차(24시간 공백 뒤 새로 셈 · feed 는 2의 거듭제곱 번째) · 24시간 복귀 발화 수.
        "edge_episode": g.edge_episode,
        "edge_returns_24h": g.edge_returns_within(now, 86_400.0),
        "clears_24h": clears_24h,
        "confirmed_clears_24h": confirmed_24h,
        // ★(통합 minor 정리 · R1V42-3) 효과 없음 회차(24시간 공백 뒤 새로 셈 · feed 는 2의 거듭제곱 번째) · 24시간 효과 없음 수.
        "ineffective_episode": g.ineff_episode,
        "ineffective_24h": g.ineffective_within(now, 86_400.0),
        // ★(통합 minor 정리 · V42R-3) 위 회차·24시간 수가 세어진 시작 = 이 데몬 세대의 기동 시각(epoch) — 가드 상태는 휘발이라 재기동
        //   뒤 0 부터 센다(24시간 수는 min(24시간, 이 시각 뒤)치).
        "counts_since": daemon.started_at,
    })
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
    // ★(clear 가드 v3) 관측(위) 뒤에 — 시한(Awaiting·Cycling·Measuring)과 보류해 둔 실측 관측의 재판정(보고가 끊긴 좌석도 발화 · I2).
    for s in surfaces.iter().filter(|s| !s.exited.load(Ordering::Relaxed)) {
        ctx_guard_tick(daemon, s);
    }
    // ★(수정 6회차 V42R-1) 비동기 사이클 대기열 — 동결 해제 뒤·자리 난 뒤 시작(접수·작업 끝에도 부르지만 놓친 것을 2초마다 줍는다).
    crate::cycle_jobs::pump(daemon);
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
        // ★(clear 가드 v3) 새 세션 파일은 가드 입력이 아니다(clear 의 증거는 사이클 표지뿐 — 헬퍼·휴리스틱 재발견이 경로를
        //   바꿔도 수준·막대·발화에 닿지 않는다). 경로는 낙폭 판정 범위 이름으로만 게이트에 실린다(`ctx_guard_scope`).
        tails.insert(s.id, TailState::attach(daemon, path.clone(), heuristic, now));
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
                    // ★0.14.43(B3): claude transcript 는 rate 를 **이월**한다(rate 의 생산자는 상태줄뿐) — 값과 함께 관측 시각·귀속 계정도 이월한다.
                    //   `updated_at: now` 는 이월에도 갱신되므로 rate 의 나이가 아니다(`ObservedUsage::rate_observed_at`).
                    let (rate, rate_observed_at, rate_account) = carried_rate(next.as_ref(), prev.as_ref());
                    next = Some(ObservedUsage {
                        agent: agent.into(),
                        ctx_tokens: Some(ctx_tokens),
                        ctx_window: Some(window),
                        ctx_pct: pct(ctx_tokens, window),
                        rate,
                        source: source_label("transcript", state.heuristic),
                        session_file: state.path.to_string_lossy().into_owned(),
                        updated_at: now,
                        rate_observed_at,
                        rate_account,
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
                    // ★0.14.43(B3): 이 틱에 rollout 이 **새로 낸** rate 면 관측 시각 = now, 이월(base.rate 복사)이면 base 의 관측 시각을 이월한다.
                    //   (codex 는 계정 귀속이 단일 홈 `default` 라 rate_account 는 두지 않는다 — 좌석 축 로그인 전환 판정은 claude 전용.)
                    let (rate, rate_observed_at) = codex_rate_merge(obs.rate, base, now);
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
                        rate_observed_at,
                        rate_account: None,
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
    // 결정론 컨텍스트 임계 — 자기보고(status.set)·statusline 과 **공유 게이트**(좌석의 clear 가드 하나 · 가드 락 하나)
    // 로 발화한다. 분리된 판정을 쓰면 같은 교차에 두 경로가 각각 발화해 master/CSO가
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

/// ★0.14.43(B3) claude transcript tail 의 rate **이월**(순수 — 핀): 이번 틱에 앞서 만든 `next` 가 있으면 그것을, 없으면 직전 스냅샷 `prev` 를 따른다. 값뿐 아니라
/// 관측 시각(`rate_observed_at`)과 귀속 계정(`rate_account`)도 함께 이월한다 — 이월에도 `updated_at` 은 now 로 갱신되므로, 관측 시각을 이월하지 않으면 유휴 좌석의 옛 rate 가
/// 신선한 것으로 둔갑한다(경보 신선도 규칙을 무력화하는 통로). 어느 쪽도 없으면 빈 rate · 관측 시각 모름(0) · 귀속 없음.
pub(crate) fn carried_rate(next: Option<&ObservedUsage>, prev: Option<&ObservedUsage>) -> (Vec<RateWindow>, f64, Option<String>) {
    match (next, prev) {
        (Some(n), _) => (n.rate.clone(), n.rate_observed_at, n.rate_account.clone()),
        (None, Some(p)) => (p.rate.clone(), p.rate_observed_at, p.rate_account.clone()),
        (None, None) => (Vec::new(), 0.0, None),
    }
}

/// ★0.14.43(B3) codex rollout 틱의 rate 병합(순수 — 핀): 이 이벤트가 **새로 낸** rate 면 관측 시각 = `now`, 없어서 `base`(이번 틱 앞선 스냅샷 또는 직전 스냅샷)의 rate 를 이월하면
/// base 의 관측 시각을 그대로 이월한다(없으면 빈 rate · 모름). codex 는 계정 귀속이 단일 홈이라 귀속 계정은 두지 않는다.
pub(crate) fn codex_rate_merge(fresh: Option<Vec<RateWindow>>, base: Option<&ObservedUsage>, now: f64) -> (Vec<RateWindow>, f64) {
    match fresh {
        Some(r) => (r, now),
        None => base.map_or((Vec::new(), 0.0), |b| (b.rate.clone(), b.rate_observed_at)),
    }
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
        // ★0.14.43(B3): agy 프로브는 항상 신선 생산 — 관측 시각 = 지금(귀속 계정은 단일 홈이라 두지 않는다).
        rate_observed_at: now_epoch(),
        rate_account: None,
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

    /// ★0.14.43(B3) transcript tail 의 rate 이월 핀: 값과 함께 **관측 시각·귀속 계정이 이월**된다(`updated_at` 이 now 로 갱신돼도 rate 의 나이는 그대로) ·
    /// 이번 틱의 앞선 스냅샷(`next`)이 직전(`prev`)보다 우선 · 둘 다 없으면 빈 rate·관측 시각 모름·귀속 없음. codex 는 새로 낸 rate 만 now, 이월은 base 의 시각.
    #[test]
    fn b3_rate_carryover_keeps_the_observation_time_and_the_attributed_account() {
        let w = |pct: f64| vec![RateWindow { label: "5h".into(), used_pct: pct, resets_at: Some(9_999_999_999.0) }];
        let snap = |pct: f64, at: f64, acct: Option<&str>, updated: f64| ObservedUsage {
            agent: "claude".into(),
            ctx_tokens: None,
            ctx_window: None,
            ctx_pct: None,
            rate: w(pct),
            source: "statusline".into(),
            session_file: String::new(),
            updated_at: updated,
            rate_observed_at: at,
            rate_account: acct.map(str::to_string),
        };
        let prev = snap(99.0, 1_000.0, Some("u-b3-a"), 5_000.0);
        let (r, at, acct) = carried_rate(None, Some(&prev));
        assert_eq!((r, at, acct.as_deref()), (w(99.0), 1_000.0, Some("u-b3-a")), "이월이 관측 시각·귀속 계정을 잃었다(updated_at 5000 이 아니라 rate 의 1000)");
        let next = snap(50.0, 2_000.0, None, 5_000.0);
        let (r, at, acct) = carried_rate(Some(&next), Some(&prev));
        assert_eq!((r, at, acct), (w(50.0), 2_000.0, None), "이번 틱의 앞선 스냅샷이 직전보다 우선이다");
        assert_eq!(carried_rate(None, None), (Vec::new(), 0.0, None));
        // codex: 새로 낸 rate 는 now · 이월은 base 의 관측 시각(now 가 아니다) · base 없음은 모름
        let base = snap(70.0, 3_000.0, None, 5_000.0);
        assert_eq!(codex_rate_merge(Some(w(10.0)), Some(&base), 7_000.0), (w(10.0), 7_000.0));
        assert_eq!(codex_rate_merge(None, Some(&base), 7_000.0), (w(70.0), 3_000.0), "codex 이월이 관측 시각을 now 로 둔갑시켰다");
        assert_eq!(codex_rate_merge(None, None, 7_000.0), (Vec::new(), 0.0));
        // 빈 새 rate(Some(빈 벡터))도 '새로 낸 것'이다(종전 `obs.rate.or_else` 와 같은 의미)
        assert_eq!(codex_rate_merge(Some(Vec::new()), Some(&base), 7_000.0), (Vec::new(), 7_000.0));
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
            rate_observed_at: 0.0,
            rate_account: None,
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
            rate_observed_at: 0.0,
            rate_account: None,
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

/// ★(0.14.42 · clear 가드 v3) 순수 상태기계 핀 — 시각은 단조 초를 직접 준다.
///   ① 단위 검체 24종: 참조 구현 검체(evidence/v3/model/tests/cg3_unit.rs)를 이름·단언 그대로 옮겼다.
///   ② 이전 반례 이름 검체: 반례의 좌석 모양을 작은 좌석 구동기([`drive`] — 1초 틱 · 대기열·턴·Claude 선제 압축·집행자 지연)의
///      사건열(report · cycle · tick)로 옮겼다. 좌석 모형 전수 스윕(evidence/v3 · 재검증자 모형 sweep_idle·busy·chan · run_seat)은
///      운영 코드를 그대로 발췌해 따로 돌린다(clear-guard 보고서).
///   ③ 성질 검체: 무작위 사건열 1만 씨앗(결정론) — I1·I2·I3(차분 오라클). 참조 구현 prop.rs 이식.
/// 통합 핀(게이트·표지·틱·claim·payload·배선)은 `handlers::tests::context_threshold_*` · `ctx_guard_*`.
#[cfg(test)]
mod ctx_guard_tests {
    use super::clear_guard::{self as cg, block_cap, stop_cap, Axis, ClearGuard, Kind, Note, Phase, Rep, Verdict, Why};
    use super::*;

    const W: Option<u64> = Some(200_000);

    fn rep(g: &mut ClearGuard, pct: u8, scope: &str, now: f64) -> cg::Out {
        g.report(&Rep { pct, window: W, axis: Axis::Measured, scope, base: 60, now, frozen: false })
    }
    fn srep(g: &mut ClearGuard, pct: u8, now: f64) -> cg::Out {
        g.report(&Rep { pct, window: W, axis: Axis::SelfReport, scope: "", base: 60, now, frozen: false })
    }
    fn fired(o: &cg::Out) -> bool {
        matches!(o.verdict, Some(Verdict::Fire { .. }))
    }
    fn strikes(o: &cg::Out) -> Vec<Why> {
        o.notes.iter().filter_map(|n| if let Note::Ineffective { why, .. } = n { Some(*why) } else { None }).collect()
    }
    /// 부트 발화 → 사이클(켬 +60 · 끔 +120) → 창 동안 `floor` 보고 → 창 닫힘 + 접기 관측. 반환: 창이 닫힌 뒤 시각.
    fn boot_and_cycle(g: &mut ClearGuard, floor: u8, scope: &str) -> f64 {
        assert!(fired(&rep(g, floor.max(61), "s0", 0.0)), "부트 첫 교차는 기본 임계에서 발화");
        g.cycle(true, 60.0, false);
        g.cycle(false, 120.0, false);
        let mut t = 130.0;
        while t < 720.0 {
            rep(g, floor, scope, t);
            t += 30.0;
        }
        let _ = g.tick(730.0, false);
        assert_eq!(g.phase, Phase::Free);
        let o = rep(g, floor, scope, 740.0); // 접기(창 최고치 범위의 첫 관측)
        assert!(!fired(&o));
        750.0
    }

    // ───────────── ① 단위 검체 24종(참조 구현 cg3_unit.rs 이식 · 이름·단언 그대로) ─────────────

    #[test]
    fn rr3_r1_1_idle_floor_does_not_refire_without_growth() {
        let mut g = ClearGuard::default();
        let mut t = boot_and_cycle(&mut g, 72, "s1");
        // 주기 신호(2분 0.15%p) 40분 — 표시값 72→73 · 막대 77
        let mut x = 72.0f64;
        while t < 750.0 + 2400.0 {
            x += 0.15 / 120.0 * 30.0;
            assert!(!fired(&rep(&mut g, x.round() as u8, "s1", t)));
            let _ = g.tick(t, false);
            t += 30.0;
        }
        assert_eq!(g.bar(Axis::Measured, 60, W), 77);
    }

    #[test]
    fn marker_is_the_only_clear_signal_session_file_change_does_nothing() {
        let mut g = ClearGuard::default();
        let t = boot_and_cycle(&mut g, 72, "s1");
        let lvl = g.level;
        // 세션 파일이 A→B→A 로 바뀌어도(헬퍼·휴리스틱 재발견) 수준·막대·상태는 그대로다.
        for (k, s) in ["s2", "s1", "s3", "s1"].iter().enumerate() {
            let o = rep(&mut g, 74, s, t + 10.0 * k as f64);
            assert!(!fired(&o));
            assert_eq!(g.phase, Phase::Free);
        }
        assert_eq!(g.level, lvl);
    }

    #[test]
    fn helper_scope_low_reports_are_not_compactions() {
        let mut g = ClearGuard::default();
        let t = boot_and_cycle(&mut g, 72, "s1");
        rep(&mut g, 75, "s1", t);
        let o = rep(&mut g, 10, "h1", t + 10.0); // 헬퍼 transcript
        assert!(strikes(&o).is_empty());
        assert_eq!(g.phase, Phase::Free, "다른 범위의 낮은 값은 압축이 아니다");
    }

    #[test]
    fn tick_never_reads_self_report_stale_value() {
        // F1: 자기보고는 도착 때만 판정 — 틱은 가드 밖 값을 읽지 않는다(보류해 둔 실측 관측만 다시 판정).
        let mut g = ClearGuard::default();
        rep(&mut g, 20, "r0", 0.0);
        for k in 1..2000 {
            let o = g.tick(k as f64 * 2.0, false);
            assert!(!fired(&o));
        }
    }

    #[test]
    fn self_report_axis_is_measured_after_cycle_by_its_own_first_report() {
        // F1b/F3: 실측 축 수준(20)이 자기보고 축 막대를 낮추지 않는다 · 사이클 뒤 첫 자기보고가 그 축 수준.
        let mut g = ClearGuard::default();
        assert!(fired(&srep(&mut g, 65, 0.0))); // 자기보고 축 부트
        g.cycle(true, 60.0, false);
        g.cycle(false, 120.0, false);
        rep(&mut g, 20, "r1", 200.0); // 실측(창 안)
        let _ = g.tick(800.0, false);
        assert!(!fired(&srep(&mut g, 65, 2000.0)), "창 뒤 첫 자기보고는 수준(접기) — 발화하지 않는다");
        assert!(!fired(&srep(&mut g, 65, 3800.0)), "같은 값 되풀이 = 성장 없음");
        assert_eq!(g.bar(Axis::SelfReport, 60, W), 70);
        assert!(fired(&srep(&mut g, 70, 5600.0)));
    }

    #[test]
    fn self_report_seat_clear_is_seen_by_marker_without_drop() {
        // F3: 자기보고 좌석은 복원 뒤에만 보고해도(낙폭 없음) 사이클 표지로 clear 를 안다 — 시한 strike 없음.
        let mut g = ClearGuard::default();
        assert!(fired(&srep(&mut g, 70, 0.0)));
        g.cycle(true, 300.0, false);
        g.cycle(false, 360.0, false);
        let _ = g.tick(1000.0, false);
        let o = srep(&mut g, 70, 1500.0);
        assert!(!fired(&o) && strikes(&o).is_empty());
        assert!(g.fire.unwrap().cleared);
    }

    #[test]
    fn timeout_is_tentative_and_late_marker_resets_strikes() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        let o = g.tick(1200.0, false); // 시한 — 잠정
        assert!(o.notes.iter().any(|n| matches!(n, Note::Unanswered { .. })));
        assert!(strikes(&o).is_empty() && g.strikes == 0);
        // 늦은 사이클(시한 뒤 · 다음 발화 전)
        g.cycle(true, 1500.0, false);
        g.cycle(false, 1560.0, false);
        assert!(g.fire.unwrap().cleared);
        let _ = g.tick(2200.0, false);
        rep(&mut g, 72, "s1", 2210.0);
        let o = rep(&mut g, 78, "s1", 4000.0);
        assert!(fired(&o) && strikes(&o).is_empty() && g.strikes == 0);
    }

    #[test]
    fn no_cycle_strike_only_at_next_fire() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        let o = g.tick(1200.0, false);
        assert!(strikes(&o).is_empty());
        // 잠정 보류 = 1200 + 900 · 그 전 재판정은 보류
        assert!(matches!(rep(&mut g, 78, "s0", 1500.0).verdict, Some(Verdict::Held { until, .. }) if (until - 2100.0).abs() < 1e-6));
        let o = g.tick(2100.0, false); // 보류 만료 — 보류해 둔 실측 관측으로 재발화
        assert!(fired(&o));
        assert_eq!(strikes(&o), vec![Why::NoCycle]);
    }

    #[test]
    fn frozen_delivery_freezes_awaiting() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        let mut t = 0.0;
        while t < 7200.0 {
            let o = g.tick(t, true);
            assert!(o.notes.is_empty());
            t += 2.0;
        }
        assert!(matches!(g.phase, Phase::Awaiting { .. }), "동결 중에는 시한이 흐르지 않는다");
        g.cycle(true, 7300.0, false);
        g.cycle(false, 7400.0, false);
        assert_eq!(g.strikes, 0);
        assert!(g.fire.unwrap().cleared);
    }

    #[test]
    fn drop_in_awaiting_resolves_without_strike() {
        let mut g = ClearGuard::default();
        rep(&mut g, 72, "s0", 0.0); // 부트 발화
        g.cycle(true, 10.0, false);
        g.cycle(false, 20.0, false);
        rep(&mut g, 72, "s1", 30.0);
        rep(&mut g, 72, "s1", 40.0);
        let _ = g.tick(700.0, false);
        rep(&mut g, 72, "s1", 710.0);
        assert!(fired(&rep(&mut g, 78, "s1", 1300.0)));
        let seq = g.seq;
        let o = rep(&mut g, 34, "s1", 1500.0); // 사이클 전에 압축
        assert!(strikes(&o).is_empty());
        assert!(o.notes.iter().any(|n| matches!(n, Note::DropBeforeCycle { .. })));
        assert!(!g.stale(seq), "압축은 건너뛸 사유가 아니다 — 늦은 사이클은 수준을 다시 잰다");
        let o = g.tick(5000.0, false);
        assert!(strikes(&o).is_empty(), "시한 strike 도 없다(발화 1건당 결과 1회)");
        // 늦게 온 사이클: 결과는 이미 정해졌다(strike 없음) · 수준은 다시 잰다.
        g.cycle(true, 5100.0, false);
        g.cycle(false, 5160.0, false);
        assert!(g.stale(seq));
        assert!(matches!(g.phase, Phase::Measuring { kind: Kind::AfterCycle, .. }));
        assert_eq!(g.strikes, 0);
    }

    #[test]
    fn drop_after_cycle_is_one_strike_per_fire() {
        let mut g = ClearGuard::default();
        let t = boot_and_cycle(&mut g, 80, "s1");
        let o = rep(&mut g, 36, "s1", t + 30.0); // clear 뒤 다음 발화 전 압축
        assert_eq!(strikes(&o), vec![Why::DropAfterCycle]);
        assert!((g.hold_until - (t + 30.0 + 900.0)).abs() < 1e-6);
        let o = rep(&mut g, 20, "s1", t + 60.0);
        assert!(strikes(&o).is_empty(), "같은 발화에 두 번째 strike 없음");
    }

    #[test]
    fn lone_stop_cap_report_is_not_level() {
        // F4: 사이클 뒤 창의 첫 보고가 S 이상(낡은 값 88) → 수준도 낙폭 기준도 아니다.
        let mut g = ClearGuard::default();
        rep(&mut g, 86, "s0", 0.0);
        g.cycle(true, 60.0, false);
        g.cycle(false, 120.0, false);
        rep(&mut g, 88, "s1", 130.0);
        let mut t = 140.0;
        while t < 720.0 {
            rep(&mut g, 72, "s1", t);
            t += 20.0;
        }
        let _ = g.tick(730.0, false);
        rep(&mut g, 72, "s1", 735.0);
        assert_eq!(g.level[0].map(|l| l.0 .0), Some(72));
        assert_eq!(g.bar(Axis::Measured, 60, W), 77);
    }

    #[test]
    fn bar_never_exceeds_block_cap() {
        // 막대 = max(기본, min(R+5, max(S, R+1), C)) — R 이 아무리 높아도 C(차단점 아래 표시값) 이하.
        for (w, c, want) in [(Some(200_000u64), 88u8, 88u8), (Some(1_000_000), 97, 96), (None, 88, 88)] {
            assert_eq!(block_cap(w), c);
            let mut g = ClearGuard::default();
            g.level[0] = Some(((95, w), Kind::AfterCycle));
            assert_eq!(g.bar(Axis::Measured, 60, w), want);
            for r in 0..=100u8 {
                g.level[0] = Some(((r, w), Kind::AfterCycle));
                assert!(g.bar(Axis::Measured, 60, w) <= c.max(60));
            }
        }
        assert_eq!((stop_cap(Some(200_000)), stop_cap(Some(1_000_000)), stop_cap(None)), (85, 96, 85));
    }

    #[test]
    fn stale_first_report_is_not_a_compaction_baseline() {
        let mut g = ClearGuard::default();
        let t = boot_and_cycle(&mut g, 72, "s1");
        // 새 범위의 첫 보고가 낡은 높은 값 → 다음 보고가 12%p 낮아도 압축이 아니다(확인 전 첫 관측).
        rep(&mut g, 76, "s9", t);
        let o = rep(&mut g, 64, "s9", t + 10.0);
        assert!(strikes(&o).is_empty());
        assert_eq!(g.phase, Phase::Free);
    }

    #[test]
    fn fold_absorbs_unobserved_tail_growth_into_level() {
        let mut g = ClearGuard::default();
        rep(&mut g, 72, "s0", 0.0);
        g.cycle(true, 60.0, false);
        g.cycle(false, 120.0, false);
        rep(&mut g, 72, "s1", 130.0);
        rep(&mut g, 72, "s1", 140.0);
        rep(&mut g, 10, "h1", 600.0); // 도구 호출(헬퍼) — 좌석 상태줄 공백
        let _ = g.tick(730.0, false);
        rep(&mut g, 10, "h1", 740.0); // 다른 범위 — 접지 않는다
        rep(&mut g, 75, "s1", 800.0); // 좌석 첫 관측 — 접는다(R = 75)
        assert_eq!(g.level[0].map(|l| l.0 .0), Some(75));
        assert!(!fired(&rep(&mut g, 79, "s1", 900.0)));
        assert!(fired(&rep(&mut g, 80, "s1", 1300.0)));
    }

    /// 보류 재판정(I2 · RR2-ROLE-1): 보류가 끝나면 틱이 그 축의 **가장 최근 관측**을 다시 판정한다 — 보고가 끊긴 좌석은 마지막
    /// 관측(71)으로 발화하고, 그 사이 다른 범위의 막대 아래 보고가 왔으면(표지 없는 clear 뒤 새 세션 · 헬퍼) 그 값이라 발화하지
    /// 않는다(가드는 헬퍼와 새 세션을 가르지 않는다 — 좌석 세션이 다시 자라면 그 보고가 곧바로 판정된다).
    #[test]
    fn held_retry_fires_after_backoff_without_new_reports() {
        let setup = || {
            let mut g = ClearGuard::default();
            let t = boot_and_cycle(&mut g, 80, "s1");
            rep(&mut g, 66, "s1", t + 30.0); // 압축(14%p 낙폭) strike → 보류 900
            let _ = g.tick(t + 200.0, false);
            rep(&mut g, 66, "s1", t + 210.0); // 접기 → R 66 · 막대 71
            let o = rep(&mut g, 71, "s1", t + 300.0);
            let until = match o.verdict {
                Some(Verdict::Held { until, .. }) => until,
                v => panic!("보류여야 한다: {v:?}"),
            };
            (g, t, until)
        };
        let (mut g, _, until) = setup();
        assert!(!fired(&g.tick(until - 1.0, false)));
        assert!(matches!(g.tick(until, false).verdict, Some(Verdict::Fire { pct: 71, .. })), "보고가 끊긴 좌석도 보류 만료에 발화(I2)");
        // 그 사이 다른 범위의 막대 아래 보고 — 가장 최근 관측이 그것이다(죽은 세션 값을 되살리지 않는다).
        let (mut g, t, until) = setup();
        rep(&mut g, 10, "h1", t + 400.0);
        assert!(!fired(&g.tick(until, false)), "가장 최근 관측(10)이 아니라 보류해 둔 값(71)으로 재발화했다");
        assert!(fired(&rep(&mut g, 72, "s1", until + 30.0)), "좌석 세션이 다시 보고하면 곧바로 판정된다");
    }

    #[test]
    fn min_spacing_600() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        g.cycle(true, 10.0, false);
        g.cycle(false, 20.0, false);
        rep(&mut g, 90, "s1", 25.0);
        rep(&mut g, 90, "s1", 30.0); // S 이상 두 번 = 확인(외톨이 짝 · 수준 밖 — V42NC-1) → 조기 닫힘 · 이 보고로 판정(막대 = 기본 · 최소 간격 보류)
        assert!(!fired(&rep(&mut g, 91, "s1", 35.0)), "최소 간격(600초) 전에 발화했다");
        let o = rep(&mut g, 91, "s1", 40.0);
        assert!(matches!(o.verdict, Some(Verdict::Held { until, .. }) if (until - 600.0).abs() < 1e-6), "{o:?}");
    }

    #[test]
    fn backoff_doubles_to_7200_cap() {
        assert_eq!([cg::backoff(1), cg::backoff(2), cg::backoff(3), cg::backoff(4), cg::backoff(9)], [900.0, 1800.0, 3600.0, 7200.0, 7200.0]);
    }

    #[test]
    fn empty_scope_name_is_just_a_scope() {
        let mut g = ClearGuard::default();
        rep(&mut g, 55, "", 0.0);
        assert!(fired(&rep(&mut g, 62, "", 10.0)));
        g.cycle(true, 380.0, false);
        rep(&mut g, 3, "", 390.0);
        g.cycle(false, 400.0, false);
        let o = rep(&mut g, 20, "", 410.0);
        assert!(strikes(&o).is_empty() && g.strikes == 0);
        assert!(matches!(g.phase, Phase::Measuring { kind: Kind::AfterCycle, .. }));
    }

    #[test]
    fn window_switch_rebases_level() {
        let mut g = ClearGuard::default();
        g.level[0] = Some(((72, Some(200_000)), Kind::AfterCycle));
        assert_eq!(g.bar(Axis::Measured, 10, Some(1_000_000)), 20); // 72%·200K = 14.4%·1M → 올림 15 + 5
    }

    /// S 가장자리 조기 닫힘(RNC5-1) — 같은 범위의 S 미만 보고가 창의 최고치면 확인된 것이다. 창을 곧바로 닫고(R = 그 전 최고치
    /// 80 · 이 보고는 수준 밖) **이 보고로 곧바로 판정한다**(막대 ≤ S = 85 · 최소 간격 전이면 보류 → 만료에 발화).
    #[test]
    fn early_close_at_stop_cap_after_confirmation() {
        let mut g = ClearGuard::default();
        rep(&mut g, 80, "s0", 0.0);
        g.cycle(true, 10.0, false);
        g.cycle(false, 20.0, false);
        rep(&mut g, 80, "s1", 30.0);
        let o = rep(&mut g, 85, "s1", 40.0); // 같은 범위의 S 미만 보고가 창에 있다 → 곧바로 닫고 이 보고로 판정
        assert_eq!(g.phase, Phase::Free);
        assert_eq!(g.level[0].map(|l| l.0 .0), Some(80), "닫힘을 부른 S 보고는 수준 밖이다");
        assert_eq!(g.fold[0], None, "S 가장자리로 닫힌 축은 접지 않는다(이 보고가 곧 판정)");
        assert!(matches!(o.verdict, Some(Verdict::Held { bar: 85, until })  if (until - 600.0).abs() < 1e-6), "{o:?}");
        assert!(o.notes.iter().any(|n| matches!(n, Note::Measured { cut: false, span, edge: Some((85, _)), .. } if (*span - 20.0).abs() < 1e-6)));
        assert!(!fired(&g.tick(599.0, false)));
        assert!(matches!(g.tick(600.0, false).verdict, Some(Verdict::Fire { pct: 85, bar: 85, .. })), "최소 간격 만료에 S 보고로 발화");
    }

    /// S 가장자리 보고는 접히지 않는다 — 같은 턴이 S 위로 이어지면(85 → 86) 그 뒤 보고도 곧바로 판정되고(보류 · 가장 최근 관측이
    /// 재판정 재료) 만료에 가장 최근 관측(86)으로 발화한다. 한 사이클 뒤 발화는 하나다(Awaiting 이 막는다).
    #[test]
    fn edge_report_is_decided_not_folded() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        g.cycle(true, 10.0, false);
        g.cycle(false, 20.0, false);
        rep(&mut g, 80, "s1", 30.0);
        assert!(matches!(rep(&mut g, 85, "s1", 38.0).verdict, Some(Verdict::Held { .. })));
        let o = rep(&mut g, 86, "s1", 46.0);
        assert!(matches!(o.verdict, Some(Verdict::Held { .. })), "접기가 아니라 판정이어야 한다: {o:?}");
        assert!(!o.notes.iter().any(|n| matches!(n, Note::MeasuredLate { .. })), "{o:?}");
        let o = g.tick(600.0, false);
        assert!(matches!(o.verdict, Some(Verdict::Fire { pct: 86, .. })), "{o:?}");
        for t in [620.0, 700.0, 900.0] {
            assert!(!fired(&rep(&mut g, 87, "s1", t)), "Awaiting 중 재발화");
        }
    }

    #[test]
    fn executor_single_flight_stale_query() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        assert!(!g.stale(1));
        g.cycle(true, 100.0, false);
        g.cycle(false, 200.0, false);
        assert!(g.stale(1), "사이클이 끝난 뒤의 같은 발화 경보는 건너뛴다");
        assert!(!g.stale(2));
        // 더 새 발화가 있다는 것만으로는 옛 경보를 건너뛰지 않는다(느린 집행자 굶음 방지) — 사이클이 끝나야 건너뛴다.
        let _ = g.tick(900.0, false);
        rep(&mut g, 78, "s1", 910.0);
        assert!(fired(&rep(&mut g, 84, "s1", 1500.0)));
        assert_eq!(g.seq, 2);
        assert!(g.stale(1) && !g.stale(2));
    }

    #[test]
    fn cycling_cap_closes_without_off() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        g.cycle(true, 100.0, false);
        let _ = g.tick(100.0 + 660.0, false);
        assert!(matches!(g.phase, Phase::Measuring { kind: Kind::AfterCycle, .. }));
        assert!(g.fire.unwrap().cleared);
    }

    #[test]
    fn measuring_window_closes_by_time_without_reports() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 78, "s0", 0.0)));
        g.cycle(true, 100.0, false);
        g.cycle(false, 200.0, false);
        let _ = g.tick(800.0, false);
        assert_eq!(g.phase, Phase::Free);
        assert_eq!(g.fold[0], Some(0), "창이 비었으면 다음 관측(아무 범위)이 수준");
    }

    // ───────────── ①-2 수정 2회차 반례 검체(RNC4-1 · R1V3-1 · RR1-ROLE-1 · RR1-ROLE-2) ─────────────

    /// RNC4-1: 발화 두 번이 무응답(CSO 부재 · clear 전 실패)으로 끝나 strikes 2 · 보류 6900 이 걸린 뒤 세 번째 발화의 사이클이
    /// 끝났다. 끝난 사이클은 집행자가 응답했다는 증거다 — 그 보류가 남으면 clear 직후 좌석이 다시 자라도 '마지막 발화 + 1800'
    /// 까지 막혀 자동 압축을 끈 좌석이 차단점(88.5)을 넘는다(②). 다음 발화는 잰 수준 위 성장(R+5 · 가장자리)에서 난다.
    /// 음성 대조: clear 안 됨(NotCleared) 표지는 사이클 끝이 아니므로 보류를 풀지 않는다.
    #[test]
    fn rnc4_1_ended_cycle_releases_the_no_cycle_backoff_hold() {
        let run = |outcome: cg::Outcome| {
            let mut g = ClearGuard::default();
            assert!(fired(&rep(&mut g, 70, "A", 0.0)));
            let mut t = 0.0;
            let mut fires = vec![];
            while t < 5100.0 {
                t += 2.0;
                if fired(&g.tick(t, false)) {
                    fires.push(t);
                }
            }
            assert_eq!(fires, vec![2100.0, 5100.0], "무응답 발화 둘");
            assert_eq!((g.strikes, g.hold_until), (2, 6900.0));
            g.cycle_on(5160.0, false);
            g.cycle_off(outcome, 5220.0, false);
            (g, t)
        };
        let (mut g, _) = run(cg::Outcome::Cleared);
        assert!(g.hold_until <= 5220.0, "끝난 사이클 뒤에도 보류 {} 가 남았다", g.hold_until);
        // 재주입 뒤 72 → 분당 1%p.
        let mut first_fire = None;
        let mut t = 5230.0;
        let mut x = 72.0f64;
        while t < 7000.0 && first_fire.is_none() {
            let o = rep(&mut g, x.round() as u8, "B", t);
            assert!(!matches!(o.verdict, Some(Verdict::Held { until, .. }) if until >= 6900.0 - 1e-6), "t={t}: {o:?}");
            if fired(&o) {
                first_fire = Some((t, x.round() as u8));
            }
            let _ = g.tick(t, false);
            t += 10.0;
            x += 1.0 / 6.0;
        }
        let (ft, fp) = first_fire.expect("사이클 뒤 다시 자란 좌석이 발화하지 않았다");
        assert!(ft < 6220.0 && fp <= 85, "차단점(88.5) 전에 R+5·가장자리에서 발화해야 한다: {ft} {fp}");
        // 음성 대조 — clear 안 됨 표지는 보류를 풀지 않는다(그 발화는 미해결).
        let (g, _) = run(cg::Outcome::NotCleared);
        assert_eq!(g.hold_until, 6900.0);
        assert!(!g.stale(3) && !g.fire.unwrap().cleared);
    }

    /// R1V3-1 → 수정 3회차(RNC5-1 · 설계 I3 예외 ⓑ 명시): 사이클 뒤 수준이 S 위로 돌아오는 좌석(자동 압축 끔 · 재주입 + 붙잡혔던
    /// 대기열 = 86)은 **사이클마다 한 번** S 가장자리로 발화한다 — 그 S 보고는 작업 덩어리가 넘긴 S 보고와 같은 관측이라(검체
    /// `edge_cannot_tell_restore_from_work_so_both_fire_every_cycle`) 접으면 작업 좌석이 차단점을 넘는다(②). 묶임: 최소 간격(600초)
    /// · 사이클이 오지 않으면 효과 없음 strike 백오프(15·30·60·120분 · I1(b)) · 막대 ≤ S.
    #[test]
    fn r1v3_1_return_above_stop_cap_fires_once_per_cycle_bounded_by_spacing() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        g.cycle_on(60.0, false);
        g.cycle_off(cg::Outcome::Cleared, 75.0, false);
        for (k, p) in [72u8, 76, 79, 82].into_iter().enumerate() {
            rep(&mut g, p, "s1", 80.0 + 8.0 * k as f64);
        }
        let o = rep(&mut g, 86, "s1", 120.0); // 대기열 배달 — S 이상(같은 범위 확인) → 창 닫힘 · 이 보고로 판정(최소 간격 보류)
        assert_eq!(g.level[0].map(|l| l.0 .0), Some(82), "S 보고는 수준 밖");
        assert!(matches!(o.verdict, Some(Verdict::Held { bar: 85, until }) if (until - 600.0).abs() < 1e-6), "{o:?}");
        // 사이클이 오지 않는 좌석(집행자 부재) — 성장 0 인 채 효과 없음 백오프로만 재통보한다.
        let mut fires = vec![];
        let mut t = 130.0;
        while t < 6.0 * 3600.0 {
            if fired(&rep(&mut g, 86, "s1", t)) {
                fires.push(t);
            }
            if fired(&g.tick(t, false)) {
                fires.push(t);
            }
            t += 2.0;
        }
        assert_eq!(fires.first().copied(), Some(600.0), "{fires:?}");
        let gaps: Vec<f64> = fires.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.windows(2).all(|w| w[1] >= w[0] - 2.0), "사이클 없는 재통보 간격이 늘지 않는다(백오프): {gaps:?}");
        assert!(fires.len() <= 6, "사이클 없는 좌석 6시간 재통보 {} > 6: {fires:?}", fires.len());
    }

    /// 가장자리(설계 I3 ⓑ): S 부터는 R+1(1%p 성장)이면 발화 · R < C 이면 막대 > R(성장 없이 발화하지 않는다 — R1V3-1 드릴 r1a 의
    /// 사이클 뒤 87% CEO) · R ≥ C 이면 막대 = C(설계 예외).
    #[test]
    fn edge_bar_requires_growth_below_the_block_cap() {
        let bar = |r: u8, w: Option<u64>| cg::bar_of(Some((r, w)), 60, w);
        assert_eq!((bar(84, W), bar(85, W), bar(86, W), bar(87, W), bar(88, W), bar(95, W)), (85, 86, 87, 88, 88, 88));
        let m = Some(1_000_000u64);
        assert_eq!((bar(94, m), bar(95, m), bar(96, m), bar(97, m)), (96, 96, 97, 97));
        for r in 0..=100u8 {
            for w in [W, m, None] {
                let c = block_cap(w);
                let b = bar(r, w);
                assert!(b <= c.max(60), "막대 {b} > C {c}");
                if r < c && r >= 60 {
                    assert!(b > r, "R {r} 에서 성장 없는 막대 {b}(창 {w:?})");
                }
            }
        }
    }

    /// RR1-ROLE-1: 표지 끔의 결과 — clear 안 됨(표지 켬 뒤 송신 거부 rc 85 · 실효 미관측 rc 80)은 사이클 끝이 아니다. 표지 켬 전
    /// 상태(Awaiting · 같은 시한)로 돌아가고, 그 발화는 풀리지 않으며(stale 거짓 → 같은 --fire 재집행) 수준을 다시 재지 않는다.
    /// 시한이 지나면 종전처럼 잠정 보류 뒤 효과 없음으로 재발화한다(보고가 끊겨도 · I2).
    #[test]
    fn not_cleared_marker_is_not_a_cycle_end() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let before = g.phase;
        let _ = g.cycle_on(400.0, false);
        let o = g.cycle_off(cg::Outcome::NotCleared, 480.0, false);
        assert!(o.notes.is_empty(), "{o:?}");
        assert_eq!(g.phase, before, "표지 켬 전 상태(같은 시한)로 돌아가야 한다");
        assert!(!g.stale(1) && !g.fire.unwrap().cleared);
        assert_eq!(g.resolved_through, 0);
        assert_eq!(g.level, [None, None], "clear 되지 않은 수준을 사이클 뒤 수준으로 재면 안 된다");
        // 재집행이 clear 에 성공하면 그때 끝이다.
        let _ = g.cycle_on(700.0, false);
        let _ = g.cycle_off(cg::Outcome::Cleared, 760.0, false);
        assert!(g.stale(1) && g.fire.unwrap().cleared);
        assert!(matches!(g.phase, Phase::Measuring { kind: Kind::AfterCycle, confirmed: true, .. }));
        // 끝내 clear 되지 않으면: 시한 → 잠정 보류 → 보류해 둔 관측으로 효과 없음 재발화.
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let _ = g.cycle_on(400.0, false);
        let _ = g.cycle_off(cg::Outcome::NotCleared, 480.0, false);
        let o = g.tick(1200.0, false);
        assert!(o.notes.iter().any(|n| matches!(n, Note::Unanswered { .. })), "{o:?}");
        let o = g.tick(2100.0, false);
        assert!(fired(&o) && strikes(&o) == vec![Why::NoCycle], "{o:?}");
        // 재는 창 도중 clear 안 됨(수동 사이클 실패)은 그 창으로 돌아간다(같은 기준 시각).
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let _ = g.cycle(true, 10.0, false);
        let _ = g.cycle_off(cg::Outcome::Cleared, 20.0, false);
        rep(&mut g, 72, "s1", 30.0);
        let _ = g.cycle_on(100.0, false);
        let _ = g.cycle_off(cg::Outcome::NotCleared, 150.0, false);
        assert!(matches!(g.phase, Phase::Measuring { anchor, .. } if (anchor - 20.0).abs() < 1e-9), "{:?}", g.phase);
        assert!(matches!(g.tick(620.0, false).notes.as_slice(), [Note::Measured { .. }]));
    }

    /// RR1-ROLE-1: 결과를 모르는 끔(측정 불능 rc 81 · 데몬 해제 · 수동 quiesce · 구 CLI)은 종전처럼 사이클 끝이지만 창은 'clear
    /// 실효 미확인'을 싣는다(feed 가 clear 를 단정·처방하지 않게) · 확인된 끔만 확인이다.
    #[test]
    fn unknown_marker_ends_the_cycle_without_confirming_the_clear() {
        for (outcome, confirmed) in [(cg::Outcome::Unknown, false), (cg::Outcome::Cleared, true)] {
            let mut g = ClearGuard::default();
            assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
            let _ = g.cycle_on(60.0, false);
            let _ = g.cycle_off(outcome, 120.0, false);
            assert!(g.stale(1), "{outcome:?}");
            rep(&mut g, 72, "s1", 130.0);
            let o = g.tick(730.0, false);
            assert!(
                o.notes.iter().any(|n| matches!(n, Note::Measured { kind: Kind::AfterCycle, confirmed: c, cut: false, .. } if *c == confirmed)),
                "{outcome:?}: {o:?}"
            );
        }
        // 구 CLI(결과 없음)·모르는 값은 모름이다(종전 동작 · 실패 방향 = 수준 재측정).
        assert_eq!(cg::Outcome::parse(None), cg::Outcome::Unknown);
        assert_eq!(cg::Outcome::parse(Some("garbage")), cg::Outcome::Unknown);
        assert_eq!(cg::Outcome::parse(Some("cleared")), cg::Outcome::Cleared);
        assert_eq!(cg::Outcome::parse(Some(" not_cleared ")), cg::Outcome::NotCleared);
    }

    /// RR1-ROLE-2: 사이클 뒤 재는 창 안에서 압축(같은 범위 10%p 낙폭)이 오면 그 창을 낙폭 전 최고치로 먼저 닫는다 — 효과 없음
    /// strike 의 '사이클 뒤 수준'은 그 창이 잰 값(83)이지 앞 창의 값·'관측 없음'이 아니고, 사이클 뒤 수준 보고(끊긴 창)도 나간다.
    #[test]
    fn drop_inside_after_cycle_window_reports_the_window_peak() {
        let mut g = ClearGuard::default();
        // 앞선 압축 창의 낮은 수준(33)이 있는 좌석 — 종전에는 이 값이 strike 문구에 실렸다.
        g.level[0] = Some(((33, W), Kind::AfterCompaction));
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)) || g.fire.is_some());
        let _ = g.cycle_on(60.0, false);
        let _ = g.cycle_off(cg::Outcome::Cleared, 100.0, false);
        let mut t = 115.0;
        let mut p = 67.6f64;
        while t < 320.0 {
            rep(&mut g, p.round() as u8, "s1", t);
            p = (p + 0.4).min(83.4);
            t += 4.6;
        }
        let o = rep(&mut g, 30, "s1", 323.3);
        let measured = o.notes.iter().find_map(|n| match n {
            Note::Measured { kind: Kind::AfterCycle, level, cut: true, span, confirmed: true, .. } => Some((level[0], *span)),
            _ => None,
        });
        assert_eq!(measured.map(|(l, _)| l), Some(Some((83, W))), "끊긴 창의 사이클 뒤 수준 보고: {o:?}");
        assert!(measured.is_some_and(|(_, s)| (s - 223.3).abs() < 1e-6));
        assert!(
            o.notes.iter().any(|n| matches!(n, Note::Ineffective { why: Why::DropAfterCycle, level: Some((83, _)), drop: Some((83, 30)), .. })),
            "{o:?}"
        );
        assert!(matches!(g.phase, Phase::Measuring { kind: Kind::AfterCompaction, .. }), "압축 뒤 재는 창(90초)");
    }

    // ───────────── 상수·범위 키·발행 재료 ─────────────

    /// 파라미터는 설계 v3 §3 값이다(모형 스윕 param-sweep-v3.txt 로 고른 값 — 바꾸면 I1·I3 증명 수치가 달라진다).
    #[test]
    fn guard_constants_are_the_design_values() {
        assert_eq!(CTX_GUARD_GROWTH, 5);
        assert_eq!(
            (CTX_GUARD_MEASURE_SECS, CTX_GUARD_REREAD_SECS, CTX_GUARD_MIN_SPACING_SECS, CTX_GUARD_CLEAR_WAIT_SECS, CTX_GUARD_CYCLING_MAX_SECS),
            (600.0, 90.0, 600.0, 1200.0, 660.0)
        );
        assert_eq!((CTX_GUARD_BACKOFF_BASE_SECS, CTX_GUARD_BACKOFF_MAX_SECS), (900.0, 7200.0));
        assert_eq!((CTX_GUARD_COMPACT_DROP, CTX_GUARD_STOP_MARGIN_TOKENS, CTX_GUARD_SCOPES_KEPT), (10, 5_000, 4));
    }

    /// 범위 = 세션 파일 줄기 — 심링크·`/private` 표기 차이는 같은 범위 · 빈 경로는 "" 범위.
    #[test]
    fn scope_is_the_session_file_stem() {
        let a = ctx_guard_scope("/Users/x/.claude/projects/p/9f3a.jsonl");
        assert_eq!(a, "9f3a");
        assert_eq!(ctx_guard_scope("/private/var/folders/q/.claude/projects/p/9f3a.jsonl"), a);
        assert_ne!(ctx_guard_scope("/Users/x/.claude/projects/p/7b21.jsonl"), a);
        assert_eq!(ctx_guard_scope(""), "");
        assert_eq!(ctx_guard_scope("   "), "");
    }

    /// 발행 재료 — 창이 닫힐 때 최고치를 낸 관측의 기본 임계를 싣고(틱이 오버라이드 파일을 다시 읽지 않는다), 효과 없음은 그
    /// 발화 번호·축 수준을 싣는다.
    #[test]
    fn notes_carry_publish_materials_without_outside_reads() {
        let mut g = ClearGuard::default();
        assert!(fired(&g.report(&Rep { pct: 70, window: W, axis: Axis::Measured, scope: "s0", base: 65, now: 0.0, frozen: false })));
        g.cycle(true, 60.0, false);
        g.cycle(false, 120.0, false);
        g.report(&Rep { pct: 72, window: W, axis: Axis::Measured, scope: "s1", base: 65, now: 130.0, frozen: false });
        let o = g.tick(720.0, false);
        assert!(
            o.notes.iter().any(|n| matches!(n, Note::Measured { kind: Kind::AfterCycle, base, level, .. }
                if base[0] == Some(65) && level[0] == Some((72, W)))),
            "{o:?}"
        );
        g.report(&Rep { pct: 72, window: W, axis: Axis::Measured, scope: "s1", base: 65, now: 730.0, frozen: false });
        let o = g.report(&Rep { pct: 40, window: W, axis: Axis::Measured, scope: "s1", base: 65, now: 800.0, frozen: false });
        assert!(
            o.notes.iter().any(|n| matches!(n, Note::Ineffective { why: Why::DropAfterCycle, fire_seq: 1, level: Some((72, _)), drop: Some((72, 40)), .. })),
            "{o:?}"
        );
    }

    // ───────────── ② 이전 반례 이름 검체 — 작은 좌석 구동기 ─────────────

    /// 좌석 한 대의 모양(시각 초 · 컨텍스트 %p). 사이클: 발화 → `exec_delay` 뒤 표지 켬(clear 3%) → `quiesce` 뒤 붙여넣기
    /// (`paste`) · 표지 끔 → 복원 턴(`restore`/`restore_secs`) → 사이클이 붙잡은 몰림(`burst` · 대기열) → 몰림 뒤 회신(`late`).
    /// 입력은 대기열로 들어오고 턴 사이 1초 틈에 한 건씩 배달된다(사이클 동안 보류). Claude 선제 압축: 턴 도중·제출 때 컨텍스트가
    /// 창 − 33000 토큰 이상이면 `keep` 으로 내리고 남은 턴 + 지침 재읽기(`reread` · 20초)를 이어 간다. 상태줄은 턴 도중 8초마다 ·
    /// 턴 끝 · 압축 1초 뒤 보고한다. 수집기 틱 2초.
    #[derive(Clone, Copy)]
    struct Seat {
        window: Option<u64>,
        base: u8,
        boot: f64,
        paste: f64,
        restore: f64,
        restore_secs: f64,
        burst: (u32, f64, f64),
        late: Option<(f64, f64, f64)>,
        hb: (f64, f64),
        work: (f64, f64, f64),
        autocompact: bool,
        keep: f64,
        reread: f64,
        exec_delay: f64,
        exec_fail: bool,
        /// 이 시각 전의 발화는 집행되지 않는다(CSO 부재·라우터 보류 · RNC4-1).
        absent_until: f64,
        /// 작업(`work`)이 이 시각부터 시작한다(그 전에는 주기 신호만).
        work_from: f64,
        quiesce: f64,
        secs: f64,
        /// (수정 4회차) 자동 압축 끔 좌석이 차단점 위면 집행의 저장 지시가 거부돼 표지 켬 전에 멈춘다(모형·드릴과 같은 실패 모양).
        /// 기본 끔 — 종전 핀은 가드 수준의 발화 지속(I2)을 단언한다(집행 실패 모양은 이 필드를 켠 핀과 모형 경계표가 잰다).
        reject_above_block: bool,
    }

    impl Seat {
        /// 200K master(붙여넣기 63.3 + 복원 8.9 = 72.2) · 주기 신호 2분 0.15%p · 집행 60초 · quiescing 15초.
        fn master200() -> Self {
            Seat {
                window: W,
                base: 60,
                boot: 70.0,
                paste: 63.3,
                restore: 8.9,
                restore_secs: 30.0,
                burst: (0, 0.0, 10.0),
                late: None,
                hb: (120.0, 0.15),
                work: (0.0, 0.0, 10.0),
                autocompact: true,
                keep: 34.0,
                reread: 30.0,
                exec_delay: 60.0,
                exec_fail: false,
                absent_until: 0.0,
                work_from: 0.0,
                quiesce: 15.0,
                secs: 4.0 * 3600.0,
                reject_above_block: false,
            }
        }
        fn growth(&self) -> f64 {
            let per = |every: f64, pp: f64, secs: f64| if every > 0.0 { pp * secs / every } else { 0.0 };
            per(self.hb.0, self.hb.1, self.secs) + per(self.work.0, self.work.1, (self.secs - self.work_from).max(0.0))
        }
    }

    #[derive(Debug, Default)]
    struct Run {
        fires: Vec<(f64, u8)>,
        clears: Vec<f64>,
        compactions: Vec<f64>,
        max_after_first_clear: f64,
        secs_above_block: f64,
        max_bar: u8,
        strikes: Vec<Why>,
        /// 가드에 준 상태줄 보고(시각, 퍼센트) · 사이클 표지 끔 시각 — 관측 동치 검체용.
        reports: Vec<(f64, u8)>,
        offs: Vec<f64>,
        /// 차단점 위에서 저장 지시가 거부돼 표지 켬 전에 멈춘 집행 수(자동 압축 끔 · 수정 4회차 — 모형·드릴과 같은 실패 모양).
        failed_cycles: usize,
    }

    impl Run {
        fn min_spacing(&self) -> f64 {
            self.fires.windows(2).map(|w| w[1].0 - w[0].0).fold(f64::INFINITY, f64::min)
        }
        fn max_hour(&self) -> usize {
            (0..self.fires.len()).map(|i| self.fires[i..].iter().take_while(|f| f.0 - self.fires[i].0 < 3600.0).count()).max().unwrap_or(0)
        }
        fn max_gap(&self, secs: f64) -> f64 {
            let mut ts: Vec<f64> = self.fires.iter().map(|f| f.0).collect();
            ts.push(secs);
            ts.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max)
        }
    }

    /// I2 상한(설계 §2): 보류 7200 + 시한 1200 + 창 600 + Cycling 60 여유.
    const I2_BOUND: f64 = 7200.0 + 1200.0 + 600.0 + 60.0;

    fn drive(seat: Seat) -> Run {
        use std::collections::VecDeque;
        let mut g = ClearGuard::default();
        let mut run = Run::default();
        let w = seat.window.unwrap_or(200_000) as f64;
        let compact_at = (w - (CC_SUMMARY_RESERVE_TOKENS + CC_AUTOCOMPACT_BUFFER_TOKENS) as f64) * 100.0 / w;
        let block_at = (w - (CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS) as f64) * 100.0 / w;
        let mut pct = seat.boot;
        let mut queue: VecDeque<(f64, f64)> = VecDeque::new();
        let mut turn: Option<(f64, f64)> = None; // (끝, 초당 성장)
        let mut last_turn_end = 0.0f64;
        let mut held = false; // quiescing — 대기열 보류
        let (mut cycle_on_at, mut cycle_off_at): (Option<f64>, Option<f64>) = (None, None);
        let mut late_at: Option<f64> = None;
        let mut sess = 0u32;
        let mut next_report = f64::INFINITY;
        let mut force_report: Option<f64> = Some(0.0);
        let (mut next_hb, mut next_work) = (seat.hb.0, seat.work.0);
        let mut t = 0.0f64;
        while t <= seat.secs {
            // 입력
            if seat.hb.0 > 0.0 && t >= next_hb {
                next_hb += seat.hb.0;
                queue.push_back((seat.hb.1, 5.0));
            }
            if seat.work.0 > 0.0 && t >= next_work {
                next_work += seat.work.0;
                if t >= seat.work_from {
                    queue.push_back((seat.work.1, seat.work.2));
                }
            }
            if late_at.is_some_and(|a| t >= a) {
                late_at = None;
                if let Some((_, pp, secs)) = seat.late {
                    queue.push_back((pp, secs));
                }
            }
            // 사이클(집행자)
            if cycle_on_at.is_some_and(|a| t >= a) {
                cycle_on_at = None;
                if seat.reject_above_block && !seat.autocompact && pct >= block_at {
                    // 차단점 — 저장 지시(대상 턴)가 거부된다: cycle-agent 는 저장 검증에서 멈춘다(표지 켬 전 · clear 없음 · 그 발화는
                    // 미해결 → 시한·잠정 보류 뒤 재발화 · 좌석은 차단점 위에 머문다 — 모형·드릴의 영구 무clear 모양).
                    run.failed_cycles += 1;
                } else {
                    let _ = g.cycle(true, t, false);
                    run.clears.push(t);
                    pct = 3.0;
                    turn = None;
                    held = true;
                    for _ in 0..seat.burst.0 {
                        queue.push_back((seat.burst.1, seat.burst.2));
                    }
                    cycle_off_at = Some(t + seat.quiesce);
                }
            }
            if cycle_off_at.is_some_and(|a| t >= a) {
                cycle_off_at = None;
                sess += 1;
                pct = seat.paste;
                turn = Some((t + seat.restore_secs, seat.restore / seat.restore_secs));
                held = false;
                let _ = g.cycle(false, t, false);
                run.offs.push(t);
                // 몰림 뒤 회신: 복원 끝 + 몰림 턴들 뒤 지연.
                if let Some((d, _, _)) = seat.late {
                    let backlog = f64::from(seat.burst.0) * (seat.burst.2 + 1.0);
                    late_at = Some(t + seat.restore_secs + backlog + d);
                }
                next_report = t + 8.0;
            }
            // Claude — 턴 진행 · 선제 압축
            if let Some((end, rate)) = turn {
                pct += rate;
                if seat.autocompact && pct >= compact_at {
                    let rem = (end - t).max(0.0);
                    pct = seat.keep;
                    let grow = rate * rem + seat.reread;
                    let e2 = end.max(t) + 20.0;
                    turn = Some((e2, grow / (e2 - t).max(1.0)));
                    run.compactions.push(t);
                    force_report = Some(t + 1.0);
                }
            }
            if turn.is_some_and(|(end, _)| t >= end) {
                turn = None;
                last_turn_end = t;
                force_report = Some(t);
            }
            if turn.is_none() && !held && t - last_turn_end >= 1.0 {
                if let Some((pp, secs)) = queue.pop_front() {
                    let mut grow = pp;
                    let mut dur = secs;
                    if seat.autocompact && pct >= compact_at {
                        pct = seat.keep;
                        grow += seat.reread;
                        dur += 20.0;
                        run.compactions.push(t);
                        force_report = Some(t + 1.0);
                    } else if !seat.autocompact && pct >= block_at {
                        grow = 0.0; // 차단점 — 프롬프트가 막힌다
                    }
                    turn = Some((t + dur, grow / dur));
                    next_report = t + 8.0;
                }
            }
            if run.clears.first().is_some_and(|c| t > *c) {
                run.max_after_first_clear = run.max_after_first_clear.max(pct);
            }
            if pct >= block_at {
                run.secs_above_block += 1.0;
            }
            // 상태줄 보고
            let due = force_report.is_some_and(|f| t >= f) || (turn.is_some() && t >= next_report);
            let mut verdicts = vec![];
            if due && cycle_off_at.is_none() {
                force_report = None;
                next_report = t + 8.0;
                let p = pct.round().clamp(0.0, 100.0) as u8;
                let scope = format!("s{sess}");
                let o = g.report(&Rep { pct: p, window: seat.window, axis: Axis::Measured, scope: &scope, base: seat.base, now: t, frozen: false });
                run.reports.push((t, p));
                verdicts.push(o);
            }
            // 수집기 틱(2초)
            if (t as u64) % 2 == 0 {
                verdicts.push(g.tick(t, false));
            }
            for o in verdicts {
                run.strikes.extend(strikes(&o));
                if let Some(Verdict::Fire { pct: p, bar, .. }) = o.verdict {
                    run.fires.push((t, p));
                    run.max_bar = run.max_bar.max(bar);
                    if !seat.exec_fail && t >= seat.absent_until && cycle_on_at.is_none() && cycle_off_at.is_none() {
                        cycle_on_at = Some(t + seat.exec_delay);
                    }
                }
            }
            run.max_bar = run.max_bar.max(g.bar(Axis::Measured, seat.base, seat.window));
            t += 1.0;
        }
        run
    }

    fn assert_i1(run: &Run, what: &str) {
        assert!(run.min_spacing() >= 600.0, "{what}: 최소 발화 간격 {} < 600 (I1)", run.min_spacing());
        assert!(run.max_hour() <= 6, "{what}: 굴림 1시간 발화 {} > 6 (I1)", run.max_hour());
    }

    fn assert_idle_bound(run: &Run, seat: &Seat, what: &str) {
        let bound = 2.0 + seat.growth() / 5.0;
        assert!(run.clears.len() as f64 <= bound, "{what}: 유휴 clear {} > {bound:.1} (I3) · 발화 {:?}", run.clears.len(), run.fires);
    }

    fn assert_keeps_firing(run: &Run, seat: &Seat, min_fires: usize, what: &str) {
        assert!(run.fires.len() >= min_fires, "{what}: 발화 {} < {min_fires} (②) · {:?}", run.fires.len(), run.fires);
        let gap = run.max_gap(seat.secs);
        assert!(gap <= I2_BOUND, "{what}: 최대 발화 간격 {gap:.0}s > {I2_BOUND} (I2 — 영구·장기 무clear) · {:?}", run.fires);
        assert!(run.max_bar <= block_cap(seat.window).max(seat.base), "{what}: 막대 {} > C (101·NEVER 없음)", run.max_bar);
    }

    /// G3ROLE-1: 느린 복원(240초)·작업 0 — 복원 성장만으로는 발화하지 않는다(부트 1회뿐).
    #[test]
    fn g3role_1_slow_restore_growth_does_not_fire() {
        let seat = Seat { restore_secs: 240.0, hb: (0.0, 0.0), secs: 3.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        assert_eq!(run.fires.len(), 1, "부트 뒤 발화 {:?}", run.fires);
        assert_i1(&run, "G3ROLE-1");
    }

    /// ROLE-R4-1·R2NC5-1: 여유 master + 사이클이 붙잡은 몰림 3×3%p + 분당 0.5%p — 차단기 오판으로 영구 무clear 가 되지 않는다.
    #[test]
    fn role_r4_1_roomy_master_with_held_backlog_keeps_clearing() {
        let seat = Seat { restore_secs: 14.0, burst: (3, 3.0, 10.0), work: (60.0, 0.5, 30.0), secs: 6.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        assert_keeps_firing(&run, &seat, 4, "ROLE-R4-1");
        assert_i1(&run, "ROLE-R4-1");
    }

    /// RV2NC-E20-1: 유휴 + 되풀이 몰림 9%p + 주기 신호 — 몰림+주기 신호 고리 없음(clear ≤ 2 + 성장/5).
    #[test]
    fn rv2nc_e20_1_idle_recurring_backlog_does_not_loop() {
        let seat = Seat { burst: (3, 3.0, 10.0), ..Seat::master200() };
        let run = drive(seat);
        assert_idle_bound(&run, &seat, "RV2NC-E20-1");
        assert_i1(&run, "RV2NC-E20-1");
    }

    /// RV2NC-E20-2: 복원 턴 도중 들어온 입력(몰림 3×3.3%p)이 복원 끝에 곧바로 이어져도 + 분당 0.55%p — 계속 clear 한다
    /// (가드 밖 채널 보류·스케줄 우회 없이).
    #[test]
    fn rv2nc_e20_2_input_during_restore_keeps_clearing() {
        let seat = Seat { burst: (3, 3.3, 10.0), work: (60.0, 0.55, 30.0), ..Seat::master200() };
        let run = drive(seat);
        assert_keeps_firing(&run, &seat, 4, "RV2NC-E20-2");
        assert_i1(&run, "RV2NC-E20-2");
    }

    /// RV2NC-E20-3: 사이클이 끝내 오지 않는 좌석(집행 실패 · CSO 부재 · 자동 압축 끔 — 압축도 없다) — 영구 정지 없이
    /// 백오프(15·30·60·120분)로 다시 통보한다. (자동 압축이 켜진 좌석은 사이클 전 압축이 그 발화의 결과를 정한다 — 효과 없음
    /// 아님 · `drop_in_awaiting_resolves_without_strike`.)
    #[test]
    fn rv2nc_e20_3_failed_cycle_refires_after_backoff() {
        let seat = Seat { exec_fail: true, autocompact: false, work: (60.0, 0.3, 20.0), secs: 12.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        assert!(run.fires.len() >= 5, "사이클 없는 좌석의 재통보 {:?}", run.fires);
        assert!(run.strikes.iter().all(|w| *w == Why::NoCycle) && !run.strikes.is_empty(), "{:?}", run.strikes);
        let gaps: Vec<f64> = run.fires.windows(2).map(|w| w[1].0 - w[0].0).collect();
        assert!(gaps.iter().all(|g| *g <= 1200.0 + 7200.0 + 2.0), "백오프 상한(시한 + 120분)을 넘는 간격: {gaps:?}");
        assert!(gaps.last().is_some_and(|g| *g >= 1200.0 + 7200.0 - 2.0), "연속 실패가 상한 백오프에 닿지 않았다: {gaps:?}");
        assert_i1(&run, "RV2NC-E20-3");
    }

    /// RV2-ROLE-1: 턴 도중 압축(480초 턴 16%p · 재읽기 33) — 압축 뒤 재측정으로 cys clear 를 계속 받는다.
    #[test]
    fn rv2_role_1_mid_turn_compaction_keeps_clearing() {
        let seat = Seat { work: (480.0, 16.0, 480.0), reread: 33.0, secs: 6.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        assert_keeps_firing(&run, &seat, 3, "RV2-ROLE-1");
        assert_i1(&run, "RV2-ROLE-1");
    }

    /// ADV2-R1-1: 가득 찬 CEO(69.35 + 8.45 + 몰림 2×3.25 = 84.3) · 24시간 — clear→압축→재무장 고리가 백오프 상한에 묶인다.
    #[test]
    fn adv2_r1_1_truly_full_ceo_is_bounded_by_backoff() {
        let seat = Seat { paste: 69.35, restore: 8.45, burst: (2, 3.25, 10.0), reread: 33.0, secs: 24.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        let late = run.fires.iter().filter(|f| f.0 >= 12.0 * 3600.0).count();
        assert!(!run.fires.is_empty() && late <= 7, "후반 12시간 발화 {late} > 7 · {:?}", run.fires);
        assert_i1(&run, "ADV2-R1-1");
    }

    /// RV3NC-1·ROLE-R5-1·R1V3-3: 여유 master(72) + 붙잡힌 몰림 5%p + 몰림 직후 끊김 없는 긴 작업 턴(480초 · 8%p · 턴 도중 압축 ·
    /// 재읽기 33) — 69e3dda6 은 막대 101 로 영구히 잠겼다(모형 360 조합 중 168). 영구·장기 무clear 없음(I2).
    #[test]
    fn rv3nc_1_roomy_seat_long_turn_after_held_backlog_never_locks() {
        let seat = Seat {
            burst: (3, 1.67, 10.0),
            late: Some((45.0, 8.0, 480.0)),
            reread: 33.0,
            work: (480.0, 8.0, 480.0),
            secs: 6.0 * 3600.0,
            ..Seat::master200()
        };
        let run = drive(seat);
        assert_keeps_firing(&run, &seat, 3, "RV3NC-1");
        assert!(run.fires.last().is_some_and(|f| f.0 >= seat.secs - I2_BOUND), "마지막 발화가 너무 이르다(잠김) · {:?}", run.fires);
        assert_i1(&run, "RV3NC-1");
    }

    /// RV3NC-2: 붙잡힌 몰림(5%p) + 몰림 뒤 회신 1건(200초 뒤 2.9%p) · 작업 0 — 회신 하나로 매 사이클 clear 되는 고리 없음.
    #[test]
    fn rv3nc_2_late_reply_after_backlog_does_not_loop() {
        let seat = Seat { burst: (1, 5.0, 10.0), late: Some((200.0, 2.9, 10.0)), ..Seat::master200() };
        let run = drive(seat);
        assert_idle_bound(&run, &seat, "RV3NC-2");
        assert_i1(&run, "RV3NC-2");
    }

    /// RV3NC-3·ROLE-R5-4: 자동 압축을 끈 여유 master + 몰림 5%p + 분당 0.5%p — 막대가 차단점(88.5%) 아래라 차단 전에 clear 된다.
    #[test]
    fn rv3nc_3_autocompact_off_is_cleared_below_the_blocking_point() {
        let seat = Seat { autocompact: false, burst: (3, 1.67, 10.0), work: (60.0, 0.5, 60.0), secs: 6.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        assert_eq!(run.secs_above_block, 0.0, "차단점 위 체류 {}초 · {:?}", run.secs_above_block, run.fires);
        assert_keeps_firing(&run, &seat, 4, "RV3NC-3");
        assert_i1(&run, "RV3NC-3");
    }

    /// R1V3-1: 몰림 3×3%p + 60초마다 짧은 작업(약 0.39%p/분) — 사이클당 성장 2%p 마다 clear 되는 고리 없음(clear ≤ 2 + 성장/5).
    #[test]
    fn r1v3_1_small_steady_work_is_bounded_by_growth() {
        let seat = Seat { burst: (3, 3.0, 10.0), work: (60.0, 0.39, 10.0), secs: 3.0 * 3600.0, ..Seat::master200() };
        let run = drive(seat);
        assert_idle_bound(&run, &seat, "R1V3-1");
        assert_i1(&run, "R1V3-1");
    }

    /// R1V3-2: 같은 발화의 두 번째 배달(라우터 보류 뒤 · 사이클이 이미 끝났다)은 stale — 집행자가 건너뛴다. 사이클 도중·Awaiting
    /// 동안의 재교차는 새 발화가 아니다.
    #[test]
    fn r1v3_2_duplicate_delivery_after_the_cycle_is_stale() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 81, "s0", 0.0)));
        assert!(!fired(&rep(&mut g, 84, "s0", 300.0)), "Awaiting 중 재교차가 새 발화가 됐다");
        g.cycle(true, 500.0, false);
        assert!(!fired(&rep(&mut g, 33, "s0", 510.0)));
        g.cycle(false, 530.0, false);
        assert!(g.stale(1), "사이클이 끝난 뒤 도착한 같은 경보를 다시 집행한다(중복 사이클)");
        assert!(!fired(&rep(&mut g, 64, "s1", 600.0)), "재주입 직후 수준(창 안)에서 기본 임계 재발화");
    }

    /// R1V3-1·R2V3-1·R3V3-1 ④(수치 재잠금 · 오너 결재 기본값 ② 우선): 자동 압축을 끈 200K 좌석 가운데 사이클 뒤 수준이 S 위로 돌아오는
    /// CEO200(77.8 + 몰림 3×3 = 86.8) · master200(72.2 + 몰림 2×2.5 + 회신 8%p = 85.2) · 실측 C 띠(재주입 88.2)는 설계 I3
    /// 예외 ⓑ·ⓒ 로 사이클마다 한 번 발화한다 — 그 보고는 작업이 넘긴 보고와 같은 관측이라(검체 `edge_cannot_tell_restore_from_work_so_
    /// both_fire_every_cycle`) 실측 축은 줄이지 않는다. 수치 상한: 24시간 clear ≤ 144(최소 간격 · 부트 포함) · 차단점 위 0초 · 발화는
    /// 전부 S 이상 관측. 고착 자기보고(C 이상 상수 · 사이클 전후 같은 값)는 백오프 상한(24시간 15) 이하다(종전 144). 이 고리는
    /// 오너 feed 가 고리 회차의 1·2·4·8…번째에 알린다(`context.edge_return` · 수정 5회차 R1V4-1).
    #[test]
    fn r1v3_1_edge_and_stuck_loops_are_numerically_bounded() {
        let d24 = 24.0 * 3600.0;
        let rej = |s: Seat| Seat { reject_above_block: true, ..s };
        let ceo = rej(Seat { paste: 69.35, restore: 8.45, burst: (3, 3.0, 10.0), hb: (0.0, 0.0), autocompact: false, secs: d24, ..Seat::master200() });
        let master = rej(Seat { burst: (2, 2.5, 10.0), late: Some((240.0, 8.0, 10.0)), hb: (0.0, 0.0), autocompact: false, secs: d24, ..Seat::master200() });
        // 실측 C 띠: 재주입만으로 88.0(+복원 0.2) — 사이클 뒤 첫 두 보고가 88·88(확인된 짝 R 88 · 막대 C 88).
        let cband = rej(Seat { paste: 88.0, restore: 0.2, hb: (0.0, 0.0), autocompact: false, secs: d24, ..Seat::master200() });
        for (name, seat) in [("CEO200 몰림3×3", ceo), ("master200 몰림2×2.5+회신8", master), ("실측 C 띠 88.2", cband)] {
            let run = drive(seat);
            assert_eq!((run.secs_above_block, run.failed_cycles), (0.0, 0), "{name}: 차단점 위 체류·저장 거부");
            assert_i1(&run, name);
            assert!(run.clears.len() <= 144, "{name}: 24시간 clear {} > 144(최소 간격 상한)", run.clears.len());
            assert!(run.fires.iter().skip(1).all(|f| f.1 >= stop_cap(W)), "{name}: S 아래 재발화 {:?}", run.fires);
        }
        let bound = backoff_bound(d24);
        for v in [89u8, 100] {
            let (fires, cycles, _) = drive_self(&|_, _| v, 60.0, cg::Outcome::Cleared, d24);
            assert!(cycles <= bound, "고착 자기보고 {v}: 24시간 clear {cycles} > 백오프 상한 {bound} · {fires:?}");
        }
        // (수정 5회차 · V41NC-1) C(88) 상수 자기보고는 정직한 재성장과 관측이 같아 고착 strike 밖 — 가장자리·C 띠 고리(② 우선)와 같은
        // 최소 간격 상한(24시간 ≤ 144 · strike 0).
        let (fires, cycles, k) = drive_self(&|_, _| 88, 60.0, cg::Outcome::Cleared, d24);
        assert!(cycles <= 144 && k == 0, "C 상수 자기보고: 24시간 clear {cycles} > 144 또는 strike {k} · {:?}", &fires[..fires.len().min(6)]);
    }

    /// RNC4-1(좌석): 집행자가 처음 두 번 부재(무응답 발화 둘 → strikes 2)였다가 복귀해 clear 에 성공한 뒤 작업이 분당 1%p 로
    /// 재개되는 자동 압축 끔 master — 효과 없음 보류가 끝난 사이클 뒤에 남으면 차단점을 넘는다(종전: 5220 clear → 6220 88.5 → 6900 발화).
    #[test]
    fn rnc4_1_seat_returning_after_absence_is_cleared_below_the_blocking_point() {
        let seat = Seat {
            boot: 75.0,
            hb: (0.0, 0.0),
            autocompact: false,
            absent_until: 5000.0,
            work_from: 5200.0,
            work: (30.0, 0.5, 20.0),
            secs: 4.0 * 3600.0,
            ..Seat::master200()
        };
        let run = drive(seat);
        assert!(run.clears.first().is_some_and(|c| *c >= 5000.0), "복귀 뒤 첫 clear {:?}", run.clears);
        assert_eq!(run.secs_above_block, 0.0, "차단점 위 체류 {}초 · 발화 {:?} · clear {:?}", run.secs_above_block, run.fires, run.clears);
        assert!(run.clears.len() >= 4, "복귀 뒤 작업 좌석이 계속 clear 되지 않는다 {:?}", run.clears);
        assert_i1(&run, "RNC4-1");
    }

    /// 차단점 근처로 돌아오는 좌석(CEO200 사이클 뒤 86.8 · 느린 작업 분당 0.1%p + 주기 신호 · 자동 압축 끔 · 집행 60초): 성장이 표시
    /// 88(C)에 닿는 대로 발화해 계속 clear 된다(I2 — 영구 무clear 없음 · 최소 간격에 묶임).
    #[test]
    fn near_block_seat_with_slow_growth_keeps_clearing() {
        let seat = Seat {
            paste: 69.35,
            restore: 8.45,
            burst: (3, 3.0, 10.0),
            autocompact: false,
            work: (20.0, 0.1 * 20.0 / 60.0, 10.0),
            secs: 6.0 * 3600.0,
            ..Seat::master200()
        };
        let run = drive(seat);
        assert_keeps_firing(&run, &seat, 20, "차단점 근처 느린 성장");
        assert!(run.clears.len() >= 20, "clear {} — 멈췄다", run.clears.len());
        assert_i1(&run, "차단점 근처 느린 성장");
    }

    // ───────────── ②-2 수정 3회차 반례 검체(RNC5-1 · R1V3-1/R2V3-1 관측 동치 · RR2-ROLE-1) ─────────────

    /// RNC5-1: 자동 압축을 끈 200K master(재주입 72.2 + 붙잡힌 몰림 3×3 = 81.2) · 600초마다 4%p 작업 덩어리 · 주기 신호 0 · 12시간.
    /// 사이클 뒤 재는 창 안에서 덩어리 하나가 S(85)를 넘으면 **그 보고로 곧바로 판정**해야 한다 — S→차단점(88.5) 여유 3.5%p 가
    /// 덩어리 하나(4%p)보다 작아서, 그 보고를 수준에 접고 R+1 을 기다리면 다음 덩어리가 차단점을 한 번에 넘고(저장 지시 거부 →
    /// 사이클 rc1) 끝까지 막힌다(a3832314: 1237초부터 12시간 끝까지 · e09a6af1: clear 72 · 차단 0 — 모형 r2sweep 동일 모양).
    #[test]
    fn rnc5_1_sparse_chunks_on_autocompact_off_seat_are_cleared_below_the_blocking_point() {
        let seat = Seat {
            autocompact: false,
            burst: (3, 3.0, 10.0),
            hb: (0.0, 0.0),
            work: (600.0, 4.0, 10.0),
            secs: 12.0 * 3600.0,
            ..Seat::master200()
        };
        let run = drive(seat);
        assert_eq!(
            run.secs_above_block, 0.0,
            "차단점 위 체류 {}초 · 발화 {:?}",
            run.secs_above_block,
            &run.fires[..run.fires.len().min(8)]
        );
        assert_keeps_firing(&run, &seat, 60, "RNC5-1");
        assert_i1(&run, "RNC5-1");
    }

    /// 관측 동치(RNC5-1 ↔ R1V3-1·R2V3-1): 세계 A(위 좌석 — 작업 덩어리가 사이클 뒤 창 안에서 S 를 넘는다)와 세계 B(같은 좌석 ·
    /// 작업 0 · 복원 끝에 붙잡혔던 회신 4%p 하나 — 유휴)는 사이클 뒤 S 교차까지 **같은 관측값 열**을 가드에 준다(재주입 66→72 ·
    /// 몰림 75·78·81 · 84·85). 창 안 관측의 시각은 판정 입력이 아니다(W = 표지 끔 뒤 600초의 모든 관측 · 복원과 작업을 가르지
    /// 않는다 — 설계 §2 규칙 4). 그러므로 S 교차 보고의 판정은 두 세계가 같고, A 가 차단점 전에 clear 되려면 그 보고로 발화해야
    /// 하므로 B 는 사이클마다 한 번 발화한다 — 설계 I3 예외 ⓑ(S 가장자리 · 최소 간격에 묶여 ≤ 6/시간 · 차단점 위 0초). B 를 줄이는
    /// 규칙(S 보고를 수준에 접기 — a3832314 · 사이클 뒤 제자리 복귀 strike — R2V3-1/2 제안)은 A 를 차단점 너머로 보낸다(수정 3회차
    /// 기각안 측정 · 오너 결재 항목). B 의 비용은 오너 feed 가 처방한다(1M · 지침 축소 · 자동 압축 켬).
    #[test]
    fn edge_cannot_tell_restore_from_work_so_both_fire_every_cycle() {
        let a = Seat {
            autocompact: false,
            burst: (3, 3.0, 10.0),
            hb: (0.0, 0.0),
            work: (600.0, 4.0, 10.0),
            secs: 3.0 * 3600.0,
            ..Seat::master200()
        };
        let b = Seat { work: (0.0, 0.0, 10.0), late: Some((0.0, 4.0, 10.0)), ..a };
        let (ra, rb) = (drive(a), drive(b));
        // 첫 사이클 표지 끔 뒤 S(85) 이상 첫 보고까지의 관측값 열.
        let prefix = |r: &Run| -> Vec<u8> {
            let off = r.offs[0];
            let mut v = vec![];
            for &(t, p) in r.reports.iter().filter(|(t, _)| *t > off) {
                v.push(p);
                if p >= stop_cap(W) || t > off + CTX_GUARD_MEASURE_SECS {
                    break;
                }
            }
            v
        };
        assert_eq!(prefix(&ra), prefix(&rb), "두 세계가 가드에 다른 관측값을 줬다(검체 전제)");
        assert!(prefix(&ra).last().is_some_and(|p| *p >= stop_cap(W)), "전제: 사이클 뒤 창 안 S 교차 {:?}", prefix(&ra));
        for (name, r) in [("A 작업 덩어리", &ra), ("B 복원 회신(유휴)", &rb)] {
            assert_eq!(r.secs_above_block, 0.0, "{name}: 차단점 위 체류");
            assert_i1(r, name);
        }
        assert!(ra.fires.len() >= 15 && rb.fires.len() >= 15, "S 교차마다 발화(가장자리): A {} · B {}", ra.fires.len(), rb.fires.len());
    }

    /// RR2-ROLE-1: 표지 없는 clear(오너 손 /clear · 에이전트 재기동 · rc 80 뒤 늦게 발효한 /clear) 뒤 새 세션이 막대 아래에서
    /// 보고하면, 시한·잠정 보류 뒤의 보류 재판정은 **그 축의 가장 최근 관측**(새 세션)을 판정한다 — 죽은 세션의 값(71)을 되살려
    /// 막 비운 좌석을 다시 clear 하지 않는다(종전: 2107초 71% 재발화 + no_cycle strike · 좌석 39% · 무응답이면 백오프마다 되풀이).
    /// 새 세션이 한 번만 보고하고 멈춰도 같다(표지 없는 clear 뒤 유휴 좌석 — 상태줄 1회).
    #[test]
    fn rr2_role_1_retry_after_unmarked_clear_judges_the_latest_observation() {
        for (name, marker, every) in [("오너 손 /clear", false, 30.0), ("rc80 뒤 늦은 발효", true, 30.0), ("새 세션 보고 1회", false, 1e9)] {
            let mut g = ClearGuard::default();
            assert!(fired(&rep(&mut g, 70, "A", 5.9)), "{name}");
            rep(&mut g, 71, "A", 200.0);
            if marker {
                let _ = g.cycle_on(300.0, false);
                let _ = g.cycle_off(cg::Outcome::NotCleared, 380.0, false);
            }
            let mut next = 401.0;
            let mut x = 26.7f64;
            let mut t = 400.0;
            while t < 12.0 * 3600.0 {
                if t >= next {
                    let o = rep(&mut g, x.round() as u8, "B", t);
                    assert!(!fired(&o), "{name} t={t}: {o:?}");
                    next += every;
                    x = (x + 0.02).min(41.8);
                }
                let o = g.tick(t, false);
                assert!(!fired(&o), "{name} t={t}: 죽은 세션의 보류 값으로 재발화 {o:?}");
                t += 2.0;
            }
        }
    }

    /// RR2-ROLE-2: 재배달은 미해결 발화(Awaiting — 표지 켬 전 · clear 안 됨 끔 뒤)에만 · 발화 1건당 한 번 · 발화가 아니다(seq·
    /// last_fire·strikes 무변경). 사이클 도중(Cycling)·끝난 뒤(Measuring·Free)·시한이 지난 뒤(잠정 보류 — 재발화가 맡는다)는 없다.
    #[test]
    fn redeliver_only_while_the_fire_is_unresolved() {
        let is_notice = |o: &cg::Out, seq: u64| matches!(o.verdict, Some(Verdict::Fire { seq: s, .. }) if s == seq);
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let (seq, last) = (g.seq, g.last_fire);
        let o = g.redeliver(100.0, false);
        assert!(is_notice(&o, 1), "Awaiting 미해결 발화는 재배달: {o:?}");
        assert_eq!((g.seq, g.last_fire, g.strikes), (seq, last, 0), "재배달이 발화처럼 셌다");
        assert!(g.redeliver(110.0, false).verdict.is_none(), "같은 발화를 두 번 재배달했다");
        // 새 발화 → 사이클 도중·끝난 뒤는 재배달 없음.
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let _ = g.cycle_on(60.0, false);
        assert!(g.redeliver(70.0, false).verdict.is_none(), "사이클 도중 재배달");
        let _ = g.cycle_off(cg::Outcome::Cleared, 120.0, false);
        assert!(g.redeliver(130.0, false).verdict.is_none(), "끝난 사이클 뒤 재배달(중복 사이클)");
        // clear 안 됨 끔 → Awaiting 복귀 → 재배달 1회.
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let _ = g.cycle_on(60.0, false);
        let _ = g.cycle_off(cg::Outcome::NotCleared, 120.0, false);
        assert!(is_notice(&g.redeliver(130.0, false), 1));
        // 시한이 지난 뒤(Free · 잠정 보류)는 재배달이 아니라 보류 재판정이 맡는다.
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        assert!(g.redeliver(1300.0, false).verdict.is_none(), "시한 뒤 재배달");
    }

    /// RR2-ROLE-1(음성 대조 · I2): 보고가 끊긴 좌석의 재통보는 그대로다 — 가장 최근 관측이 막대 이상이면 보류 만료에 발화한다.
    #[test]
    fn retry_still_fires_when_the_latest_observation_is_above_the_bar() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "A", 0.0)));
        rep(&mut g, 72, "A", 300.0); // Awaiting — 가장 최근 관측 72
        assert!(!fired(&g.tick(2099.0, false)));
        let o = g.tick(2100.0, false);
        assert!(matches!(o.verdict, Some(Verdict::Fire { pct: 72, .. })), "현재 관측(72)으로 재발화해야 한다: {o:?}");
    }

    // ───────────── ②-3 수정 4회차 반례 검체(RNC6-1 · R3V3-1) ─────────────

    /// RNC6-1(순수 API · 재검증자 U4): 사이클 뒤 창(600초)이 조용히 닫혀(R 73) 그 축을 접기로 둔 뒤, 첫 작업 보고가 한 번에 86%
    /// (S 이상)면 **접지 않고 접기 전 막대(78)로 곧바로 판정한다**(발화). 종전(e09a6af1~02bc0079)은 그 보고를 R 에 접어(R 86 · 막대
    /// 87) 발화하지 않았고, 자동 압축을 끈 200K 좌석은 다음 덩어리로 차단점(88.5)을 한 번에 넘어 저장 지시가 거부됐다(②). 접기
    /// 기회는 소비되고(그 뒤 보고도 판정) S 이상 관측은 수준에 싣지 않는다. 음성 대조: S 미만 첫 관측은 종전대로 접는다
    /// (`fold_absorbs_unobserved_tail_growth_into_level`).
    #[test]
    fn rnc6_1_first_report_at_or_above_stop_cap_after_the_window_is_decided_not_folded() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        g.cycle_on(300.0, false);
        g.cycle_off(cg::Outcome::Cleared, 360.0, false);
        for (t, p) in [(362.0, 68u8), (400.0, 72), (700.0, 73), (959.0, 73)] {
            assert!(!fired(&rep(&mut g, p, "s1", t)));
        }
        let _ = g.tick(962.0, false);
        assert_eq!(g.phase, Phase::Free);
        assert_eq!(g.fold[0], Some(cg::scope_id("s1")), "창 최고치 범위의 다음 관측을 접기로 둔다");
        let o = rep(&mut g, 86, "s1", 1500.0);
        assert!(matches!(o.verdict, Some(Verdict::Fire { pct: 86, bar: 78, .. })), "창 뒤 첫 S 이상 관측이 접혔다(RNC6-1): {o:?}");
        assert!(!o.notes.iter().any(|n| matches!(n, Note::MeasuredLate { .. })), "{o:?}");
        assert!(o.notes.iter().any(|n| matches!(n, Note::EdgeReturn { run: 1, c_band: false, .. })), "창 뒤 가장자리 발화는 복귀 계수(예외 ⓑ): {o:?}");
        assert_eq!(g.fold[0], None, "접기 기회는 소비된다");
        assert_eq!(g.level[0].map(|l| l.0 .0), Some(73), "S 이상 관측은 수준에 싣지 않는다");
    }

    /// RNC6-1(좌석 · 재검증자 좌석 검체 그대로): 자동 압축을 끈 200K master(재주입 72.2 + 붙잡힌 몰림 3×3 = 81.2) · 800초마다 3%p
    /// 작업 덩어리(한 보고 걸음) · 주기 신호 0 · 12시간. 덩어리 간격이 창(600초)보다 길고 사이 보고가 없어 창 뒤 첫 관측이 S(85)를 넘는
    /// 덩어리다 — 그 보고를 접으면(R 87 · 막대 88) 다음 덩어리(90)가 차단점을 한 번에 넘는다(02bc0079: 차단점 위 40,800초 · 모형 clear 1 ·
    /// 실패 7 · 드릴 r3-fold-head 저장 지시 거부 4). 곧바로 판정하면 차단점 위 0초.
    #[test]
    fn rnc6_1_single_step_chunks_after_the_window_are_cleared_below_the_blocking_point() {
        // 집행(표지 켬)은 발화 347초 뒤(라우터 유예·CSO 픽업 — 재검증자 모형 좌석과 같은 시각): 표지 끔 362초 → 창 962초까지 ·
        // 801초 덩어리(84)는 창 안 · 1601초 덩어리(87)가 창 뒤 첫 관측이다.
        let seat = Seat {
            autocompact: false,
            burst: (3, 3.0, 10.0),
            hb: (0.0, 0.0),
            work: (800.0, 3.0, 1.0),
            exec_delay: 347.0,
            secs: 12.0 * 3600.0,
            reject_above_block: true,
            ..Seat::master200()
        };
        let run = drive(seat);
        assert_eq!(
            (run.secs_above_block, run.failed_cycles),
            (0.0, 0),
            "차단점 위 체류 {}초 · 저장 거부 {} · 발화 {:?}",
            run.secs_above_block,
            run.failed_cycles,
            &run.fires[..run.fires.len().min(8)]
        );
        assert_keeps_firing(&run, &seat, 20, "RNC6-1");
        assert_i1(&run, "RNC6-1");
    }

    /// R3V3-1 ①②: 사이클 뒤 곧바로 가장자리(S)·C 띠로 돌아와 난 발화는 `Note::EdgeReturn`(연속 수 · 24시간 사이클 수 · 그 사이클의
    /// clear 실효 확인)을 싣는다 — 판정 무관 발행 재료(오너 error feed 1·2·4·8번째 · 좌석 행 `edge_run`·`clears_24h`). 성장으로 난
    /// 발화는 연속을 0 으로 끊는다. 부트 발화·성장 발화에는 싣지 않는다.
    #[test]
    fn edge_returns_are_counted_until_a_growth_fire() {
        let edge_of = |o: &cg::Out| {
            o.notes.iter().find_map(|n| match n {
                Note::EdgeReturn { run, cycles_24h, c_band, confirmed, .. } => Some((*run, *cycles_24h, *c_band, *confirmed)),
                _ => None,
            })
        };
        let mut g = ClearGuard::default();
        let o = rep(&mut g, 70, "s0", 0.0);
        assert!(fired(&o) && edge_of(&o).is_none(), "부트 발화는 가장자리 복귀가 아니다: {o:?}");
        let mut t = 0.0;
        for k in 1..=4u32 {
            let sc = format!("s{k}");
            g.cycle_on(t + 60.0, false);
            g.cycle_off(cg::Outcome::Cleared, t + 75.0, false);
            rep(&mut g, 72, &sc, t + 90.0);
            rep(&mut g, 80, &sc, t + 110.0);
            assert!(!fired(&rep(&mut g, 86, &sc, t + 130.0)), "최소 간격 보류");
            let o = g.tick(t + 600.0, false);
            assert!(fired(&o), "k {k}: {o:?}");
            assert_eq!(edge_of(&o), Some((k, k, false, true)), "k {k}: {o:?}");
            assert_eq!(g.edge_run, k);
            t += 600.0;
        }
        // 사이클 뒤 첫 두 보고 88·88(확인된 가장자리 짝 — 수정 6회차 V42NC-1: 짝은 수준 밖 · 창 안 S 미만 관측이 없어 수준 미상 ·
        // 막대 = 기본) · 결과 모름 끔 → 가장자리 복귀(C 띠가 아니다 — 잰 수준이 없다).
        g.cycle_on(t + 60.0, false);
        g.cycle_off(cg::Outcome::Unknown, t + 75.0, false);
        rep(&mut g, 88, "s9", t + 90.0);
        rep(&mut g, 88, "s9", t + 100.0);
        assert_eq!(g.level[0], None, "짝을 수준에 넣었다(V42NC-1)");
        let o = g.tick(t + 600.0, false);
        assert_eq!(edge_of(&o), Some((5, 5, false, false)), "{o:?}");
        t += 600.0;
        // 성장 발화(사이클 뒤 창이 S 아래에서 시각으로 닫히고 접기 뒤 G 성장) — 연속이 끊긴다.
        g.cycle_on(t + 60.0, false);
        g.cycle_off(cg::Outcome::Cleared, t + 75.0, false);
        rep(&mut g, 62, "sA", t + 90.0);
        let _ = g.tick(t + 680.0, false);
        rep(&mut g, 62, "sA", t + 700.0); // 접기
        let o = rep(&mut g, 67, "sA", t + 1300.0);
        assert!(fired(&o) && edge_of(&o).is_none(), "{o:?}");
        assert_eq!(g.edge_run, 0);
        assert_eq!(g.cycles_within(t + 1300.0, 86_400.0), (6, 5));
        // 사이클 없이 난 재통보(무응답 · C 띠 수준이어도)는 복귀 발화가 아니고 연속도 바꾸지 않는다.
        let mut g2 = ClearGuard::default();
        g2.level[0] = Some(((88, W), Kind::AfterCycle));
        let o = rep(&mut g2, 88, "x", 0.0);
        assert!(fired(&o) && edge_of(&o).is_none(), "{o:?}");
        let o = g2.tick(2100.0, false);
        assert!(fired(&o) && edge_of(&o).is_none() && g2.edge_run == 0, "무응답 재통보를 복귀 발화로 셌다: {o:?}");
        assert_eq!(g.cycles_within(t + 1300.0 + 86_400.0, 86_400.0).0, 0, "24시간 밖 사이클은 세지 않는다");
        // C 띠(잰 수준 ≥ C) 복귀 — 사이클 뒤 창에서는 S 이상 관측이 수준에 들지 않으므로(V42NC-1) 압축 뒤 창 등이 잰 수준이 C 이상일
        // 때만이다(발행 재료 c_band). 사이클이 끝난 뒤의 발화이고 수준 ≥ C 면 c_band 로 센다.
        let mut g3 = ClearGuard::default();
        assert!(fired(&rep(&mut g3, 70, "s0", 0.0)));
        g3.cycle_on(60.0, false);
        g3.cycle_off(cg::Outcome::Cleared, 75.0, false);
        let _ = g3.tick(700.0, false);
        g3.level[0] = Some(((88, W), Kind::AfterCompaction));
        g3.fold[0] = None;
        let o = rep(&mut g3, 88, "s1", 720.0);
        assert_eq!(edge_of(&o), Some((1, 1, true, true)), "{o:?}");
    }

    /// R1V4-1(수정 5회차 · 순수 API): 오너 error feed 의 재료인 **고리 회차**(`Note::EdgeReturn.episode`)는 성장 발화로 끊기지 않고
    /// 24시간 동안 복귀 발화가 없을 때만 1 부터 다시 센다 — 무작위 사건열(가장자리 복귀 · 성장 발화 · 0~30시간 공백을 섞은 5만 사이클)에서
    /// 회차가 2의 거듭제곱인 복귀 발화(= error feed)는 어느 굴림 24시간에도 ≤ 8건이다(I1 이 24시간 복귀 발화를 ≤ 144 로 묶고 회차 재시작은
    /// 24시간 공백을 요구한다). 연속 수(`run`)는 성장 발화가 0 으로 끊는 정보값이다(종전 feed 기준 — 번갈이 좌석에서 늘 1).
    #[test]
    fn r1v4_1_edge_episode_survives_growth_fires_and_restarts_only_after_a_24h_gap() {
        let edge_of = |o: &cg::Out| {
            o.notes.iter().find_map(|n| match n {
                Note::EdgeReturn { run, episode, returns_24h, .. } => Some((*run, *episode, *returns_24h)),
                _ => None,
            })
        };
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        let mut t = 0.0; // 직전 발화 시각
        let mut k = 0u32;
        // 가장자리 복귀 발화(사이클 뒤 72·80·86 → 최소 간격 만료에 발화) · 성장 발화(62 → 접기 62 → 67).
        let edge = |g: &mut ClearGuard, t: &mut f64, k: &mut u32| -> cg::Out {
            *k += 1;
            let sc = format!("e{k}");
            g.cycle_on(*t + 60.0, false);
            g.cycle_off(cg::Outcome::Cleared, *t + 75.0, false);
            rep(g, 72, &sc, *t + 90.0);
            rep(g, 80, &sc, *t + 110.0);
            assert!(!fired(&rep(g, 86, &sc, *t + 130.0)));
            let o = g.tick(*t + 600.001, false);
            assert!(fired(&o), "{o:?}");
            *t += 600.001;
            o
        };
        let growth = |g: &mut ClearGuard, t: &mut f64, k: &mut u32, gap: f64| -> cg::Out {
            *k += 1;
            let sc = format!("g{k}");
            g.cycle_on(*t + 60.0, false);
            g.cycle_off(cg::Outcome::Cleared, *t + 75.0, false);
            rep(g, 62, &sc, *t + 90.0);
            let _ = g.tick(*t + 680.0, false);
            rep(g, 62, &sc, *t + 700.0);
            let o = rep(g, 67, &sc, *t + 1300.0 + gap);
            assert!(fired(&o), "{o:?}");
            *t += 1300.0 + gap;
            o
        };
        // ① 번갈이 — 회차 1·2·3 이 이어지고 연속 수는 늘 1.
        for n in 1..=3u32 {
            let o = edge(&mut g, &mut t, &mut k);
            assert_eq!(edge_of(&o), Some((1, n, n)), "{o:?}");
            assert!(edge_of(&growth(&mut g, &mut t, &mut k, 0.0)).is_none());
        }
        // ② 24시간 이내 공백(성장 발화까지 23시간)은 회차를 끊지 않는다 · 24시간 넘는 공백 뒤 첫 복귀는 회차 1.
        assert!(edge_of(&growth(&mut g, &mut t, &mut k, 23.0 * 3600.0 - 2000.0)).is_none());
        let o = edge(&mut g, &mut t, &mut k);
        assert_eq!(edge_of(&o).map(|e| e.1), Some(4), "24시간 안의 복귀는 같은 회차: {o:?}");
        assert!(edge_of(&growth(&mut g, &mut t, &mut k, 25.0 * 3600.0)).is_none());
        let o = edge(&mut g, &mut t, &mut k);
        assert_eq!(edge_of(&o), Some((1, 1, 1)), "24시간 공백 뒤 첫 복귀는 회차 1: {o:?}");
        assert_eq!(g.edge_episode, 1);
        // ③ 무작위 5만 사이클 — 굴림 24시간 feed(회차 2의 거듭제곱) ≤ 8 · 회차 재시작 ⇔ 직전 복귀 발화가 24시간보다 오래.
        let mut r = Rng(0xC1EA_F175);
        let mut feeds: Vec<f64> = vec![];
        let mut last_edge: Option<f64> = Some(t);
        let mut prev_ep = g.edge_episode;
        for _ in 0..50_000 {
            let o = match r.pick(10) {
                0..=5 => edge(&mut g, &mut t, &mut k),
                6..=8 => growth(&mut g, &mut t, &mut k, 0.0),
                _ => growth(&mut g, &mut t, &mut k, r.f() * 30.0 * 3600.0),
            };
            if let Some((_, ep, r24)) = edge_of(&o) {
                let restarted = last_edge.is_none_or(|l| t - l > 86_400.0);
                assert_eq!(ep, if restarted { 1 } else { prev_ep + 1 }, "회차 규칙 t={t}");
                assert!(r24 >= 1 && r24 <= 145, "24시간 복귀 수 {r24}");
                if ep.is_power_of_two() {
                    feeds.push(t);
                }
                prev_ep = ep;
                last_edge = Some(t);
            }
        }
        let worst = (0..feeds.len()).map(|i| feeds[i..].iter().take_while(|x| **x - feeds[i] < 86_400.0).count()).max().unwrap_or(0);
        assert!(worst <= 8 && feeds.len() > 50, "굴림 24시간 error feed 최대 {worst} > 8(스톰) · feed {}", feeds.len());
    }

    /// V41R-2(수정 5회차 · 연관 minor): 가장자리 표시는 그 축의 **막대 아래 판정**이 끝낸다 — 가장자리 보고가 최소 간격 보류 중일 때 오너
    /// 손 /clear(표지 없음)로 새 세션이 막대 아래로 보고하면(판정 Quiet) 표시가 지워져, 몇 시간 뒤 새 세션의 성장 발화(86 ≥ 막대 85 =
    /// R80+5)는 복귀 발화가 아니다(EdgeReturn 없음 · 연속·회차 0). 종전 표시가 남아 '가장자리 복귀' error feed 와 처방이 나갔다(오진).
    /// 음성 대조: 표시 없는 같은 경로(재검증자 시나리오 C)와 결과가 같다.
    #[test]
    fn v41r_2_quiet_decision_ends_the_edge_mark() {
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "A", 0.0)));
        g.cycle_on(60.0, false);
        g.cycle_off(cg::Outcome::Cleared, 75.0, false);
        rep(&mut g, 72, "B", 90.0);
        rep(&mut g, 80, "B", 110.0);
        assert!(matches!(rep(&mut g, 86, "B", 130.0).verdict, Some(Verdict::Held { .. })), "전제: 가장자리 보고 최소 간격 보류");
        assert!(matches!(rep(&mut g, 5, "C", 200.0).verdict, Some(Verdict::Quiet)), "표지 없는 clear 뒤 새 세션");
        let mut t = 202.0;
        while t < 700.0 {
            assert!(!fired(&g.tick(t, false)));
            t += 2.0;
        }
        for (p, at) in [(20u8, 1800.0), (40, 3000.0), (60, 4200.0), (75, 5400.0), (84, 6300.0)] {
            assert!(!fired(&rep(&mut g, p, "C", at)));
        }
        let o = rep(&mut g, 86, "C", 6600.0);
        assert!(fired(&o), "새 세션 성장 발화: {o:?}");
        assert!(!o.notes.iter().any(|n| matches!(n, Note::EdgeReturn { .. })), "표지 없는 clear 뒤 성장 발화를 가장자리 복귀로 셌다(오진): {o:?}");
        assert_eq!((g.edge_run, g.edge_episode), (0, 0));
    }

    /// ★(통합 minor 정리 · R1V42-1 · 발행 전용) 가장자리 표시는 그 보고의 **범위**에 묶인다 — 다른 범위(헬퍼·claude -p 의 낮은 상태줄)의
    /// 막대 아래 판정 한 건이 본체의 표시를 지우지 않고, 자기보고 축처럼 범위가 하나인 좌석의 ±2 흔들림(86/84)은 '잰 수준 R 이 있고 통보
    /// 퍼센트 < R + G'(성장을 기다리지 않은 발화 = 예외 ⓑ·ⓒ 의 정의)로 복귀에 든다. 판정(발화 시각·수)은 바뀌지 않는다. V41R-2 모양(표지
    /// 없는 clear 뒤 다른 범위의 성장 발화)은 여전히 복귀가 아니다(검체 `v41r_2_quiet_decision_ends_the_edge_mark`). 실패 방향: 붉어지면 계속
    /// 도는 가장자리 고리가 복귀 계수·error feed·좌석 행에서 사라진다(오너 결재 ① 의 가시성 조건 — 종전 헬퍼 한 건으로 24시간 144 → 0).
    #[test]
    fn r1v42_1_edge_mark_is_scoped_and_below_growth_fires_count_as_returns() {
        let edge_of = |o: &cg::Out| o.notes.iter().any(|n| matches!(n, Note::EdgeReturn { .. }));
        // ⓐ 실측 · 헬퍼 한 건 — R 80(86 ≥ R + G 라 수치 기준은 닿지 않는다 · 범위에 묶인 표시만이 복귀를 센다).
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "body", 0.0)));
        g.cycle_on(60.0, false);
        g.cycle_off(cg::Outcome::Cleared, 75.0, false);
        rep(&mut g, 72, "body", 90.0);
        rep(&mut g, 80, "body", 110.0);
        assert!(matches!(rep(&mut g, 86, "body", 130.0).verdict, Some(Verdict::Held { .. })), "전제: 가장자리 보고 최소 간격 보류");
        assert!(matches!(rep(&mut g, 15, "helper", 200.0).verdict, Some(Verdict::Quiet)), "헬퍼 세션의 낮은 상태줄");
        let mut t = 202.0;
        while t < 700.0 {
            assert!(!fired(&g.tick(t, false)));
            t += 2.0;
        }
        let o = rep(&mut g, 86, "body", 720.0);
        assert!(fired(&o) && edge_of(&o), "헬퍼 한 건이 본체 가장자리 고리를 복귀 계수에서 지웠다: {o:?}");
        assert_eq!((g.edge_run, g.edge_episode), (1, 1));
        // ⓑ 자기보고 · 86/84 흔들림(범위 "" 하나 — 흔들림 84 가 같은 범위의 막대 아래 판정이라 표시를 지운다) — 사이클 뒤 창 72·83·86(가장자리)
        //    → 84 → 86 발화: R 83 · 86 < 88(수치 기준)만이 복귀를 센다.
        let mut g = ClearGuard::default();
        assert!(fired(&srep(&mut g, 70, 0.0)));
        g.cycle_on(60.0, false);
        g.cycle_off(cg::Outcome::Cleared, 75.0, false);
        srep(&mut g, 72, 90.0);
        srep(&mut g, 83, 110.0);
        assert!(matches!(srep(&mut g, 86, 130.0).verdict, Some(Verdict::Held { .. })));
        assert!(matches!(srep(&mut g, 84, 400.0).verdict, Some(Verdict::Quiet)), "흔들림 84 는 막대 85 아래");
        let o = srep(&mut g, 86, 720.0);
        assert!(fired(&o) && edge_of(&o), "흔들림 한 건이 자기보고 가장자리 고리를 복귀 계수에서 지웠다(R 83 · 86 < 88): {o:?}");
        // ⓒ 대조 — 사이클 뒤 G 성장으로 난 발화(R 80 · 86 ≥ 85)는 표시가 없으면 복귀가 아니다.
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "a", 0.0)));
        g.cycle_on(60.0, false);
        g.cycle_off(cg::Outcome::Cleared, 75.0, false);
        rep(&mut g, 72, "a", 90.0);
        rep(&mut g, 80, "a", 110.0);
        let _ = g.tick(700.0, false);
        rep(&mut g, 80, "a", 710.0); // 접기
        let o = rep(&mut g, 86, "a", 1300.0);
        assert!(fired(&o) && !edge_of(&o), "성장 발화를 복귀로 셌다(오진): {o:?}");
        assert_eq!(g.edge_run, 0);
    }

    /// ★(게이트 수정 1회차 ROLE-G2 · 발행 재료 — 판정 무관) 사이클 뒤 잰 수준 R 이 81~84(200K · 설계가 정상이라 적은 68~84 대역)인 좌석이
    /// **가장자리 관측 없이**(창은 S 아래에서 시각으로 닫히고 창 뒤 첫 보고도 S 아래 — 접힘) 작업으로 1~4%p 자라 막대 max(S, R+1) = 85 에서
    /// 난 **성장 발화**는 가장자리 복귀가 아니다(`Note::EdgeReturn` 없음 → 오너 error feed 없음 · 연속·회차 0) — 실측·자기보고 두 축 모두.
    /// 종전(ddad22d9)은 '통보 퍼센트 < R + G'만으로 복귀로 세어 회차 1 error feed 와 일어나지 않은 원인('창 안 가장자리 도달 · 창 뒤 첫
    /// 보고가 가장자리 이상 · 또는 C 이상')을 냈다(재검토자 growth_edge: R 81·82·83·84 모두 Some((1, 1, false))). 대조: R 78·80(성장 5%p)
    /// 은 원래 복귀가 아니고, 같은 범위가 가장자리에 닿은 뒤 흔들렸다 난 발화(자기보고 86/84 · R1V42-1 ⓑ)는 여전히 복귀다. 발화 시각·수
    /// (판정)는 바뀌지 않는다. 실패 방향: 붉어지면 정상 대역의 성장 발화마다 오진 error feed 가 나간다(오너 feed 오진 · 결재 ① 비용 과대).
    #[test]
    fn role_g2_growth_fire_from_r81_to_r84_is_not_an_edge_return() {
        let edge_of = |o: &cg::Out| o.notes.iter().find_map(|n| match n {
            Note::EdgeReturn { run, episode, c_band, .. } => Some((*run, *episode, *c_band)),
            _ => None,
        });
        for self_axis in [false, true] {
            for (r, step) in [(78u8, 0.5f64), (80, 0.5), (81, 0.5), (82, 0.5), (83, 0.5), (84, 0.25)] {
                let mut g = ClearGuard::default();
                let say = |g: &mut ClearGuard, pct: u8, now: f64| if self_axis { srep(g, pct, now) } else { rep(g, pct, "body", now) };
                assert!(fired(&say(&mut g, 70, 0.0)));
                g.cycle_on(60.0, false);
                g.cycle_off(cg::Outcome::Cleared, 90.0, false);
                // 창(90..690): 72 → R(S 미만만) — 가장자리 보고 없음.
                let (mut t, mut p) = (150.0, 72.0f64);
                while t < 690.0 {
                    let o = say(&mut g, (p.round() as u8).min(r), t);
                    assert!(edge_of(&o).is_none() && !fired(&o));
                    p += (f64::from(r) - 72.0) / 8.0;
                    t += 60.0;
                }
                let _ = g.tick(692.0, false); // 창 닫힘
                // 창 뒤 첫 보고 R(S 미만 → 접힘) · 그 뒤 작업 성장.
                let mut p = f64::from(r);
                t = 750.0;
                let mut fire = None;
                while t < 4_000.0 && fire.is_none() {
                    let o = say(&mut g, p.round().min(99.0) as u8, t);
                    let _ = g.tick(t + 1.0, false);
                    if let Some(Verdict::Fire { pct, bar, level, .. }) = o.verdict {
                        fire = Some((pct, bar, level, edge_of(&o)));
                    }
                    p += step;
                    t += 60.0;
                }
                let (pct, bar, level, edge) = fire.expect("성장 발화");
                assert_eq!(level, Some(r), "전제: 잰 수준 R");
                if r >= 81 {
                    assert_eq!((pct, bar), (85, 85), "전제: 막대 max(S, R+1) = 85 · 통보 퍼센트 < R + G");
                }
                assert_eq!(edge, None, "축 {} R {r}: 가장자리 관측 없는 성장 발화({pct}% · 막대 {bar})를 가장자리 복귀로 셌다(오진 error feed)",
                           if self_axis { "자기보고" } else { "실측" });
                assert_eq!((g.edge_run, g.edge_episode), (0, 0));
            }
        }
        // 대조 — 같은 범위가 사이클 뒤 창 안에서 가장자리(86)에 닿은 뒤 84 로 흔들렸다 86 에서 난 발화는 여전히 복귀다(R1V42-1 ⓑ · R 83).
        let mut g = ClearGuard::default();
        assert!(fired(&srep(&mut g, 70, 0.0)));
        g.cycle_on(60.0, false);
        g.cycle_off(cg::Outcome::Cleared, 75.0, false);
        srep(&mut g, 72, 90.0);
        srep(&mut g, 83, 110.0);
        assert!(matches!(srep(&mut g, 86, 130.0).verdict, Some(Verdict::Held { .. })));
        assert!(matches!(srep(&mut g, 84, 400.0).verdict, Some(Verdict::Quiet)));
        let o = srep(&mut g, 86, 720.0);
        assert!(fired(&o) && edge_of(&o) == Some((1, 1, false)), "가장자리에 닿았다 흔들린 좌석의 발화를 복귀에서 뺐다: {o:?}");
        // 대조 — 가장자리 관측 뒤 **새 창**(다음 사이클)이 S 아래에서 닫히면 그 뒤 성장 발화는 복귀가 아니다(가장자리 기록은 창마다 새로).
        g.cycle_on(780.0, false);
        g.cycle_off(cg::Outcome::Cleared, 800.0, false);
        srep(&mut g, 82, 900.0);
        let _ = g.tick(1_410.0, false);
        srep(&mut g, 82, 1_500.0); // 접기
        let o = srep(&mut g, 85, 2_000.0);
        assert!(fired(&o) && edge_of(&o).is_none(), "앞 사이클의 가장자리 기록이 다음 사이클의 성장 발화를 복귀로 셌다: {o:?}");
    }

    /// ★(통합 minor 정리 · R1V42-3 · 발행 재료) 효과 없음 회차 — 효과 있는 발화가 연속 수(strikes)를 0 으로 돌려도 회차는 끊기지 않고,
    /// 직전 효과 없음이 24시간보다 오래됐을 때만 1 부터 다시 센다. 경보를 둘 중 하나만 집행하는 좌석(strike 1 이 되풀이)의 오너 feed(회차가
    /// 2의 거듭제곱일 때만)는 24시간 ≤ 8건이다(종전 연속 수 기준 21건). 연속 효과 없음은 회차 = 연속 수(종전 feed 와 같다).
    #[test]
    fn r1v42_3_ineffective_episode_survives_effective_fires_and_restarts_after_a_24h_gap() {
        let eps = |o: &cg::Out| -> Vec<(u32, u32, u32)> {
            o.notes
                .iter()
                .filter_map(|n| match n {
                    Note::Ineffective { strikes, episode, ineffective_24h, .. } => Some((*strikes, *episode, *ineffective_24h)),
                    _ => None,
                })
                .collect()
        };
        let mut g = ClearGuard::default();
        let (mut fires, mut feeds, mut events) = (0usize, 0usize, vec![]);
        let mut pct = 30.0f64;
        let (mut on_at, mut off_at): (Option<f64>, Option<f64>) = (None, None);
        let mut t = 0.0f64;
        while t <= 24.0 * 3600.0 {
            if on_at.is_some_and(|a| t >= a) {
                g.cycle_on(t, false);
                on_at = None;
                off_at = Some(t + 60.0);
            }
            if off_at.is_some_and(|a| t >= a) {
                g.cycle_off(cg::Outcome::Cleared, t, false);
                off_at = None;
                pct = 30.0;
            }
            let mut outs = vec![];
            if (t as u64) % 30 == 0 {
                pct = (pct + 0.5).min(99.0);
                outs.push(rep(&mut g, pct.round() as u8, "a", t));
            }
            if (t as u64) % 2 == 0 {
                outs.push(g.tick(t, false));
            }
            for o in outs {
                for e in eps(&o) {
                    events.push(e);
                    if e.1.is_power_of_two() {
                        feeds += 1;
                    }
                }
                if fired(&o) {
                    fires += 1;
                    if fires % 2 == 0 && on_at.is_none() && off_at.is_none() {
                        on_at = Some(t + 120.0); // 둘 중 하나만 집행
                    }
                }
            }
            t += 1.0;
        }
        assert!(events.len() >= 15, "전제: 간헐 집행자 좌석의 효과 없음이 되풀이된다: {events:?}");
        assert!(events.iter().all(|e| e.0 == 1), "전제: 효과 있는 발화가 연속 수를 끊는다(늘 strike 1): {events:?}");
        assert!(events.windows(2).all(|w| w[1].1 == w[0].1 + 1), "회차가 효과 있는 발화로 끊겼다: {events:?}");
        assert_eq!(events.last().map(|e| e.2), Some(events.len() as u32), "24시간 효과 없음 수: {events:?}");
        assert!(feeds <= 8, "오너 feed(회차 2의 거듭제곱) 24시간 {feeds}건 > 8: {events:?}");
        // 24시간 공백 뒤에는 1 부터 다시 센다 — 부재 집행자(연속 효과 없음)는 회차 = 연속 수.
        let mut g2 = ClearGuard::default();
        assert!(fired(&rep(&mut g2, 72, "b", 0.0)));
        let mut seen = vec![];
        let mut t = 2.0;
        let mut p = 72u8;
        while t < 30.0 * 3600.0 {
            let o = g2.tick(t, false);
            seen.extend(eps(&o));
            if (t as u64) % 600 == 0 {
                p = (p + 1).min(84);
                seen.extend(eps(&rep(&mut g2, p, "b", t)));
            }
            t += 2.0;
        }
        assert!(seen.len() >= 4 && seen.iter().all(|e| e.0 == e.1), "연속 효과 없음은 회차 = 연속 수: {seen:?}");
    }

    /// 자기보고 좌석 구동기(실측 축 없음 — agy·grok 류): status.set 이 `every` 초마다 `v(시각, 끝난 사이클 수)` 를 보고한다(사이클 도중
    /// 보고는 가드가 버린다). 집행자는 발화 60초 뒤 표지 켬 · 30초 뒤 끔(결과 `outcome`). 반환: (발화 시각, 사이클 수, 최대 strikes).
    fn drive_self(v: &dyn Fn(f64, usize) -> u8, every: f64, outcome: cg::Outcome, secs: f64) -> (Vec<f64>, usize, u32) {
        let mut g = ClearGuard::default();
        let (mut fires, mut cycles, mut max_k) = (vec![], 0usize, 0u32);
        let (mut on_at, mut off_at): (Option<f64>, Option<f64>) = (None, None);
        let mut next = 1.0;
        let mut t = 0.0f64;
        while t <= secs {
            let mut outs = vec![];
            if on_at.is_some_and(|a| t >= a) {
                on_at = None;
                outs.push(g.cycle_on(t, false));
                off_at = Some(t + 30.0);
            }
            if off_at.is_some_and(|a| t >= a) {
                off_at = None;
                outs.push(g.cycle_off(outcome, t, false));
                cycles += 1;
            }
            if t >= next {
                next += every;
                outs.push(srep(&mut g, v(t, cycles), t));
            }
            if (t as u64) % 2 == 0 {
                outs.push(g.tick(t, false));
            }
            for o in outs {
                if let Some(Verdict::Fire { strikes, .. }) = o.verdict {
                    fires.push(t);
                    max_k = max_k.max(strikes);
                    if on_at.is_none() && off_at.is_none() {
                        on_at = Some(t + 60.0);
                    }
                }
            }
            max_k = max_k.max(g.strikes);
            t += 1.0;
        }
        (fires, cycles, max_k)
    }

    /// 백오프만으로 묶인 24시간 발화 상한 — 부트 1 + 900·1800·3600 뒤 7200 간격(사이클 시간 0 가정 · 실제는 그보다 적다).
    fn backoff_bound(secs: f64) -> usize {
        let (mut n, mut t, mut k) = (1usize, 0.0f64, 1u32);
        loop {
            t += cg::backoff(k);
            if t > secs {
                return n;
            }
            n += 1;
            k += 1;
        }
    }

    /// R3V3-1 ③(고착 자기보고 · 수정 5회차 V41NC-1 로 범위 조정): 자기보고 축 발화 뒤 사이클이 끝났는데 **사이클 뒤 첫 자기보고가 발화
    /// 때와 정확히 같고 차단점 이상으로만 보이는 값**(200K·미상 89 이상)이면 그 사이클은 그 축에서 효과 증거가 없다 — strike +1(지수
    /// 백오프 900·2^(k−1) ≤ 7200 · 영구 정지 없음 · I2). 그런 값을 상수로 보고하는 좌석은 종전 24시간 144회 clear 됐다 — 백오프 상한(15)
    /// 이하로 준다. C(88) 이하는 정직한 재성장과 같은 관측이라 이 규칙 밖이다(`v41nc_1_*` · 가장자리·C 띠 고리 · ② 우선).
    /// 가장자리 띠 85~87 상수는 사이클 뒤 첫 두 보고가 확인된 가장자리 짝이라(수정 6회차 V42NC-1 — 짝은 수준 밖) 최소 간격 고리다(≤ 144).
    #[test]
    fn r3v3_1_stuck_self_report_backs_off_instead_of_looping() {
        let bound = backoff_bound(86_400.0);
        assert_eq!(bound, 15, "백오프 상한 계산(0 · 900 · 2700 · 6300 · 13500 + 7200·k ≤ 86400)");
        for outcome in [cg::Outcome::Cleared, cg::Outcome::Unknown] {
            for every in [60.0, 300.0] {
                for v in [89u8, 90, 95, 100] {
                    let (fires, cycles, k) = drive_self(&|_, _| v, every, outcome, 86_400.0);
                    let what = format!("v {v} · every {every} · {outcome:?}");
                    assert!(cycles <= bound, "{what}: 고착 자기보고 24시간 clear {cycles} > 백오프 상한 {bound} · 발화 {fires:?}");
                    assert!(k >= 3, "{what}: strike 가 쌓이지 않았다(k {k})");
                    // I2 — 영구 정지 없음: 발화 간격 ≤ 백오프 상한 + 사이클 90 + 보고 간격 + 여유.
                    let gaps: Vec<f64> = fires.windows(2).map(|w| w[1] - w[0]).collect();
                    assert!(
                        gaps.iter().all(|g| *g <= CTX_GUARD_BACKOFF_MAX_SECS + 90.0 + every + 2.0 * every + 10.0),
                        "{what}: 백오프 상한을 넘는 간격(영구 정지) {gaps:?}"
                    );
                    assert!(fires.len() >= 8, "{what}: 발화가 멈췄다 {fires:?}");
                }
            }
            // 가장자리 값(85~87)을 창(600초)보다 드물게(900초) 보고하는 좌석 — 사이클 뒤 창이 비어 창 뒤 첫 보고가 S 이상이면 접지 않고
            // 판정한다(RNC6-1 · R 미상 → 막대 = 기본). 느린 정직한 보고(복원이 같은 값에 닿는 좌석)와 같은 관측이라 고착 규칙을 쓰지 않는다
            // (② 우선 · 예외 ⓑ) — 수치 상한: 보고 간격마다 1회(900초면 24시간 ≤ 96) · strike 없음. 간격이 창 끝(발화 + 사이클 + 600초)
            // 바로 뒤면 보고마다 발화한다(수정 5회차 문구 정정 · 재검증 R1V4-3: 700초 124) — 어느 간격이든 최소 간격 상한(≤ 144) 안이다.
            for v in [85u8, 86, 87] {
                let (fires, cycles, k) = drive_self(&|_, _| v, 900.0, outcome, 86_400.0);
                assert!(cycles <= 96 && k == 0, "가장자리 {v} · 900초 · {outcome:?}: 24시간 clear {cycles} > 96 또는 strike {k} · {fires:?}");
                for every in [610.0, 700.0, 1200.0, 1800.0] {
                    let (fires, cycles, k) = drive_self(&|_, _| v, every, outcome, 86_400.0);
                    assert!(cycles <= 144 && k == 0, "가장자리 {v} · {every}초 · {outcome:?}: 24시간 clear {cycles} > 144 또는 strike {k} · {:?}", &fires[..fires.len().min(6)]);
                }
            }
        }
    }

    /// R3V3-1 ③(음성 대조 · ② 회귀 0): 사이클 뒤 자기보고 값이 **바뀌면** 정상 규칙이다 — 정직한 자기보고 좌석(clear 뒤 30%에서 분당
    /// 1%p 로 자란다 · C 띠로 돌아와도 값이 다르다)은 strike 없이 성장대로 통보된다. C 아래 같은 값(복원이 우연히 발화 값에 닿은
    /// 좌석 — 설계 핀 F3 70 · 가장자리 86·87)도 strike 없다. 실측 축은 이 규칙을 쓰지 않는다(관측
    /// 동치 — `r1v3_1_edge_and_stuck_loops_are_numerically_bounded` 의 실측 C 띠 좌석).
    #[test]
    fn r3v3_1_honest_self_report_is_untouched() {
        // 정직(clear 뒤 30 → 분당 1%p · 100 상한): 발화마다 사이클이 끝나고 다음 첫 보고는 30 대 — 발화 값과 다르다.
        let last_cycle_at = std::cell::Cell::new(0.0f64);
        let seen = std::cell::Cell::new(0usize);
        let honest = |t: f64, c: usize| -> u8 {
            if c != seen.get() {
                seen.set(c);
                last_cycle_at.set(t);
            }
            (30.0 + (t - last_cycle_at.get()) / 60.0).min(100.0) as u8
        };
        let (fires, _, k) = drive_self(&honest, 60.0, cg::Outcome::Cleared, 12.0 * 3600.0);
        assert_eq!(k, 0, "정직한 자기보고에 strike: 발화 {fires:?}");
        assert!(fires.len() >= 12, "정직한 자기보고 좌석의 통보가 줄었다(②): {fires:?}");
        // C 띠로 돌아오되 값이 바뀌는 좌석(발화 89 → 사이클 뒤 88 → 다음 사이클 뒤 89 …) — 고착이 아니다(strike 0 · 예외 ⓒ 그대로).
        let alt = |_t: f64, c: usize| -> u8 { if c % 2 == 0 { 89 } else { 88 } };
        let (fires, _, k) = drive_self(&alt, 60.0, cg::Outcome::Cleared, 6.0 * 3600.0);
        assert_eq!(k, 0, "값이 바뀌는 C 띠 자기보고를 고착으로 셌다 · 발화 {fires:?}");
        assert!(fires.len() >= 20, "{fires:?}");
        // S 아래 같은 값(70) — strike 없음 · 발화는 종전과 같이 1회(부트 · 막대 = R+5).
        let (fires, _, k) = drive_self(&|_, _| 70, 60.0, cg::Outcome::Cleared, 6.0 * 3600.0);
        assert_eq!((k, fires.len()), (0, 1), "S 아래 70: {fires:?}");
        // 가장자리 띠 같은 값(86 · 87 · 60초 보고) — strike 없음. 사이클 뒤 첫 두 보고가 확인된 가장자리 짝이라(수정 6회차 V42NC-1 —
        // 짝은 수준 밖) 최소 간격마다 통보된다(예외 ⓑ · ② 우선 · 오너 결재 ①의 가장자리 고리 비용 ≤ 6/시간). 종전(짝 R = v · 막대
        // v+1)은 1회였고, 그 대가로 복원 착지가 가장자리인 정직한 좌석은 덩어리 하나로 차단점을 넘어 영구 무clear 였다
        // (`v42nc_1_self_report_edge_pair_seat_is_cleared_below_the_blocking_point`).
        for v in [86u8, 87] {
            let (fires, _, k) = drive_self(&|_, _| v, 60.0, cg::Outcome::Cleared, 6.0 * 3600.0);
            assert_eq!(k, 0, "가장자리 {v}: strike · {fires:?}");
            assert!(fires.len() >= 20 && fires.len() <= 37, "가장자리 {v}: 6시간 발화 {} — 최소 간격 고리(≤ 6/시간)가 아니다 · {fires:?}", fires.len());
        }
    }

    // ───────────── ②-4 수정 5회차 반례 검체(V41NC-1) ─────────────

    /// V41NC-1(순수 API · 재검증자 v41unit 모양): 자기보고 축 88(C) 통보 → cys 사이클 → 창 뒤 첫 자기보고가 **다시 88** 이다. C 는
    /// 차단점(200K 88.5) 아래로 보이는 가장 큰 값이라, 자동 압축을 끈 좌석이 창보다 드물게 참값을 보고하면 복원 뒤 재성장으로 발화
    /// 값과 정확히 같은 첫 보고가 나온다(주기 좌석은 매 사이클) — 고착과 관측이 같다. 그러므로 고착 strike 는 **차단점 이상으로만 보이는
    /// 값**(200K·미상 89 · 1M 99)에만 쓴다: 88 은 곧바로 판정(발화 · strike 없음). 060075e8 은 여기서 strike · 보류 1900 이었고 자기보고
    /// 축은 보류 만료에 재판정되지 않아, 그 사이 차단점을 넘은 좌석은 보고조차 못 해 영구 무clear 였다(②).
    /// 1M 창: C 97 · 98 은 참값 97.5~97.7 로 차단점(97.7) 전일 수 있다 → strike 없음 · 99 부터 strike.
    #[test]
    fn v41nc_1_honest_regrowth_to_c_after_a_cycle_is_decided_not_struck() {
        let run = |window: Option<u64>, v: u8| {
            let mut g = ClearGuard::default();
            let sr = |g: &mut ClearGuard, pct: u8, now: f64| g.report(&Rep { pct, window, axis: Axis::SelfReport, scope: "", base: 60, now, frozen: false });
            assert!(fired(&sr(&mut g, v, 0.0)));
            g.cycle(true, 60.0, false);
            g.cycle(false, 120.0, false);
            (sr(&mut g, v, 1000.0), g)
        };
        for (window, v) in [(None, 88u8), (W, 88), (Some(1_000_000), 97), (Some(1_000_000), 98)] {
            let (o, g) = run(window, v);
            assert!(strikes(&o).is_empty() && g.strikes == 0, "창 {window:?} · {v}: 정직한 재성장과 같은 관측인데 고착 strike: {o:?}");
            assert!(matches!(o.verdict, Some(Verdict::Fire { pct, .. }) if pct == v), "창 {window:?} · {v}: 곧바로 판정(발화)해야 한다: {o:?}");
        }
        // 차단점 이상으로만 보이는 값(200K·미상 89 · 1M 99) — 사이클 전후 같으면 고착 strike(백오프 · 영구 정지 없음).
        for (window, v) in [(None, 89u8), (W, 89), (W, 100), (Some(1_000_000), 99)] {
            let (o, g) = run(window, v);
            assert_eq!(strikes(&o), vec![Why::SelfReportUnchanged], "창 {window:?} · {v}: {o:?}");
            assert!(matches!(o.verdict, Some(Verdict::Held { .. })) && g.strikes == 1, "창 {window:?} · {v}: {o:?}");
        }
    }

    /// 정직한 자기보고 좌석 구동기(자동 압축 끔 · 실측 축 없음): 참값 x 는 사이클 뒤 `land` 로 돌아와 초당 `rate` %p 로 자란다. 차단점
    /// (창 − 23K)에 닿으면 제출이 막혀 성장·보고가 멈추고, 집행의 저장 지시가 거부돼 표지 켬 전에 멈춘다(모형·드릴과 같은 실패 모양).
    /// 자기보고는 `every` 초마다 round(x)(차단점 전만). 집행: 발화 60초 뒤 표지 켬 · 15초 뒤 끔(확인). 반환: (발화, 사이클, 차단점 위 초,
    /// 저장 거부 수, 최대 strikes).
    fn drive_honest_self(land: f64, rate: f64, every: f64, secs: f64) -> (Vec<(f64, u8)>, usize, f64, usize, u32) {
        let block_at = (200_000.0 - (CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS) as f64) * 100.0 / 200_000.0;
        let mut g = ClearGuard::default();
        let (mut fires, mut cycles, mut above, mut failed, mut max_k) = (vec![], 0usize, 0.0f64, 0usize, 0u32);
        let (mut on_at, mut off_at): (Option<f64>, Option<f64>) = (None, None);
        let mut x = land;
        let mut next = 1.0;
        let mut t = 0.0f64;
        while t <= secs {
            let mut outs = vec![];
            if on_at.is_some_and(|a| t >= a) {
                on_at = None;
                if x >= block_at {
                    failed += 1;
                } else {
                    outs.push(g.cycle_on(t, false));
                    x = 3.0;
                    off_at = Some(t + 15.0);
                }
            }
            if off_at.is_some_and(|a| t >= a) {
                off_at = None;
                outs.push(g.cycle_off(cg::Outcome::Cleared, t, false));
                cycles += 1;
                x = land;
            }
            if off_at.is_none() && x < block_at {
                x += rate;
            }
            if x >= block_at {
                above += 1.0;
            }
            if t >= next {
                next += every;
                if off_at.is_none() && x < block_at {
                    outs.push(srep(&mut g, x.round() as u8, t));
                }
            }
            if (t as u64) % 2 == 0 {
                outs.push(g.tick(t, false));
            }
            for o in outs {
                if let Some(Verdict::Fire { pct, .. }) = o.verdict {
                    fires.push((t, pct));
                    if on_at.is_none() && off_at.is_none() {
                        on_at = Some(t + 60.0);
                    }
                }
            }
            max_k = max_k.max(g.strikes);
            t += 1.0;
        }
        (fires, cycles, above, failed, max_k)
    }

    /// V41NC-1(좌석 · 재검증자 드릴 v41-srh 모양): 자동 압축을 끈 200K 자기보고 좌석이 복원 뒤 87.7 로 돌아와 천천히 자라고(차단점까지
    /// 보고 한 간격 + 약 25%) 창(600초)보다 드문 간격(900·1200·1800초)으로 참값을 보고한다 — 사이클 뒤 첫 보고가 매번 발화 값 88 과
    /// 같다(재성장). 그 보고로 곧바로 발화해야 차단점 전에 clear 된다: 차단점 위 0초 · 저장 거부 0 · strike 0 · 보고 간격마다 clear.
    /// 060075e8 은 그 보고를 고착으로 세어(strike · 보류) 다음 보고 전에 차단점을 넘겨 끝까지 막혔다(드릴 new-a/b 거부 59/54 ·
    /// 모형 44/4,440행). 실패 방향: 붉어지면 정직한 자기보고 좌석이 영구 무clear(②).
    #[test]
    fn v41nc_1_sparse_honest_self_report_seat_is_cleared_below_the_blocking_point() {
        let d24 = 24.0 * 3600.0;
        for every in [900.0, 1200.0, 1800.0] {
            let (fires, cycles, above, failed, k) = drive_honest_self(87.7, 0.7 / every, every, d24);
            let what = format!("보고 간격 {every}초");
            assert_eq!((above, failed, k), (0.0, 0, 0), "{what}: 차단점 위 {above}초 · 저장 거부 {failed} · strikes {k} · 발화 {:?}", &fires[..fires.len().min(6)]);
            assert!(fires.iter().all(|f| f.1 == 88), "{what}: 전제(재성장 첫 보고 = 88) {:?}", &fires[..fires.len().min(6)]);
            assert!(cycles as f64 >= d24 / every - 2.0, "{what}: 사이클 {cycles} — 보고 간격마다 clear 되지 않았다");
        }
    }

    // ───────────── ②-5 수정 6회차 반례 검체(V42NC-1) ─────────────

    /// V42NC-1(순수 API · 재검증자 v42pair 모양): 사이클 뒤 창 안에서 같은 범위의 첫 두 보고가 모두 S 이상이면(외톨이 짝 86·86) 그 짝은
    /// **확인된 가장자리 보고**다 — 단일 확인 가장자리(RNC5-1)와 같게 수준에 넣지 않고(창 안 S 미만 관측이 없으면 수준 미상 · 막대 = 기본)
    /// 그 보고로 곧바로 판정한다(막대 ≤ S · 최소 간격이면 보류 → 실측 축은 만료에 · 자기보고 축은 다음 보고에 발화). 종전(e09a6af1 ~
    /// 588fb592)은 짝의 낮은 값(86)을 수준 R 에 넣어 막대가 R+1(87)로 올랐다 — 86 이 유지되는 동안 통보가 없다가 덩어리 하나로 차단점을
    /// 넘어 저장 지시가 거부됐다(드릴 v42-pair-head-60 · 끝까지 2,883초 차단점 위 · 거부 27 · 자기보고 좌석은 보고조차 못 해 영구 무clear).
    #[test]
    fn v42nc_1_confirmed_edge_pair_is_decided_at_or_below_the_stop_cap() {
        for axis in [Axis::Measured, Axis::SelfReport] {
            let scope = if axis == Axis::Measured { "s1" } else { "" };
            let r = |g: &mut ClearGuard, pct: u8, now: f64| g.report(&Rep { pct, window: W, axis, scope, base: 60, now, frozen: false });
            let mut g = ClearGuard::default();
            assert!(fired(&r(&mut g, 70, 0.0)));
            g.cycle_on(60.0, false);
            g.cycle_off(cg::Outcome::Cleared, 75.0, false);
            assert!(matches!(r(&mut g, 86, 90.0).verdict, Some(Verdict::Quiet)), "{axis:?}: 첫 S 이상 보고는 외톨이(확인 전)");
            let o = r(&mut g, 86, 130.0);
            assert_eq!(g.phase, Phase::Free, "{axis:?}: 짝이 창을 닫는다");
            assert_eq!(g.level[axis as usize], None, "{axis:?}: 짝의 낮은 값을 수준에 넣었다(V42NC-1) — 창 안 S 미만 관측이 없으면 수준 미상");
            match o.verdict {
                Some(Verdict::Held { bar, until }) => {
                    assert!(bar <= stop_cap(W), "{axis:?}: 막대 {bar} > S — 확인된 가장자리 보고가 막혔다");
                    assert!((until - 600.0).abs() < 1e-6, "{axis:?}: {o:?}");
                }
                v => panic!("{axis:?}: 확인된 가장자리 짝은 곧바로 판정(최소 간격 보류)이어야 한다: {v:?}"),
            }
            assert!(o.notes.iter().any(|n| matches!(n, Note::Measured { edge: Some((86, _)), .. })), "{axis:?}: {o:?}");
            // 86 이 유지되는 동안 최소 간격 뒤 발화(실측 = 보류 만료 재판정 · 자기보고 = 다음 보고 — 설계 §7-8).
            let o = if axis == Axis::Measured { g.tick(600.0, false) } else { r(&mut g, 86, 660.0) };
            assert!(matches!(o.verdict, Some(Verdict::Fire { pct: 86, bar, .. }) if bar <= stop_cap(W)), "{axis:?}: 86 유지 중 통보가 없다: {o:?}");
            assert!(o.notes.iter().any(|n| matches!(n, Note::EdgeReturn { c_band: false, .. })), "{axis:?}: 짝 발화는 가장자리 복귀(예외 ⓑ): {o:?}");
        }
        // 짝의 두 값이 달라도(87 → 86) 수준 밖이다 — 창 안 S 미만 관측(다른 범위 80)이 있으면 그 값이 수준(막대 85).
        let mut g = ClearGuard::default();
        assert!(fired(&rep(&mut g, 70, "s0", 0.0)));
        g.cycle(true, 60.0, false);
        g.cycle(false, 75.0, false);
        rep(&mut g, 80, "h1", 85.0);
        rep(&mut g, 87, "s1", 90.0);
        let o = rep(&mut g, 86, "s1", 130.0);
        assert_eq!(g.level[0].map(|l| l.0 .0), Some(80), "짝은 수준 밖 — 창 안 S 미만 관측(80)이 수준");
        assert!(matches!(o.verdict, Some(Verdict::Held { bar: 85, .. })), "{o:?}");
    }

    /// V42NC-1 좌석 구동기 — 자동 압축을 끈 200K 자기보고 좌석(상태줄 없음 · agy·grok 류 또는 상태줄이 끊긴 claude): 참값 x 는 사이클 뒤
    /// `land` 로 돌아온다(복원 · LLM 은 복원 뒤에야 보고한다 — 표지 끔 뒤 첫 보고는 다음 보고 시각) · heartbeat 로 `hb.1`%p/`hb.0`초 ·
    /// 작업 덩어리 `chunk.1`%p 를 `chunk.0` 초마다(`chunk.2` 초부터 · 자기 사이클 중이면 거른다). 자기보고는 `every` 초마다 round(x)이고
    /// 차단점(창 − 23K) 이상이면 제출이 막혀 보고·성장이 멈춘다(덩어리 턴 끝 보고 1건은 나간다 — 드릴 v42-pair 와 같은 모양). 집행: 발화
    /// 60초 뒤 표지 켬(그때 x ≥ 차단점이면 저장 지시 거부 · 사이클 없음 = 실패) · 15초 뒤 끔(확인). 반환: (발화, 사이클, 차단점 위 초, 저장 거부 수).
    fn drive_chunk_self(land: f64, hb: (f64, f64), chunk: (f64, f64, f64), every: f64, secs: f64) -> (Vec<(f64, u8)>, usize, f64, usize) {
        let block_at = (200_000.0 - (CC_SUMMARY_RESERVE_TOKENS + CC_BLOCKING_BUFFER_TOKENS) as f64) * 100.0 / 200_000.0;
        let mut g = ClearGuard::default();
        let (mut fires, mut cycles, mut above, mut failed) = (vec![], 0usize, 0.0f64, 0usize);
        let (mut on_at, mut off_at): (Option<f64>, Option<f64>) = (None, None);
        let mut x = 70.0f64;
        let (mut next, mut next_hb, mut next_chunk) = (1.0, hb.0, chunk.2);
        let mut t = 0.0f64;
        while t <= secs {
            let mut outs = vec![];
            if on_at.is_some_and(|a| t >= a) {
                on_at = None;
                if x >= block_at {
                    failed += 1;
                } else {
                    outs.push(g.cycle_on(t, false));
                    x = 3.0;
                    off_at = Some(t + 15.0);
                }
            }
            if off_at.is_some_and(|a| t >= a) {
                off_at = None;
                outs.push(g.cycle_off(cg::Outcome::Cleared, t, false));
                cycles += 1;
                x = land;
            }
            let cycling = off_at.is_some();
            if t >= next_hb {
                next_hb += hb.0;
                if !cycling && x < block_at {
                    x += hb.1;
                }
            }
            if t >= next_chunk {
                next_chunk += chunk.0;
                if !cycling && x < block_at {
                    x += chunk.1;
                    if x >= block_at {
                        outs.push(srep(&mut g, x.round().min(100.0) as u8, t)); // 덩어리 턴 끝 보고(그 뒤 제출이 막힌다)
                    }
                }
            }
            if x >= block_at {
                above += 1.0;
            }
            if t >= next {
                next += every;
                if !cycling && x < block_at {
                    outs.push(srep(&mut g, x.round() as u8, t));
                }
            }
            if (t as u64) % 2 == 0 {
                outs.push(g.tick(t, false));
            }
            for o in outs {
                if let Some(Verdict::Fire { pct, .. }) = o.verdict {
                    fires.push((t, pct));
                    if on_at.is_none() && off_at.is_none() {
                        on_at = Some(t + 60.0);
                    }
                }
            }
            t += 1.0;
        }
        (fires, cycles, above, failed)
    }

    /// V42NC-1(좌석 · 재검증자 드릴 v42-pair-head-60 모양): 자기보고 60초 · 복원 착지 85.6 · heartbeat 2분 0.05%p · 1900초부터 1200초마다
    /// 2.4%p 덩어리 · 저장 거부 모형. 사이클 뒤 첫 두 보고(86·86)가 확인된 가장자리 짝이다 — 그 보고로 곧바로 판정해야 사이클마다(최소 간격)
    /// clear 되고 덩어리가 와도 차단점(88.5) 전이다: 차단점 위 0초 · 저장 거부 0. 종전은 짝을 수준(86 · 막대 87)에 넣어 86 유지 22분 동안
    /// 통보가 없다가 덩어리로 88.7 → 89 발화 → 저장 지시 거부 → 자기보고 축은 보고조차 못 해 끝까지 막혔다(드릴 거부 27 · 2,883초).
    /// 실패 방향: 붉어지면 자기보고 좌석이 영구 무clear(②).
    #[test]
    fn v42nc_1_self_report_edge_pair_seat_is_cleared_below_the_blocking_point() {
        let secs = 6.0 * 3600.0;
        let (fires, cycles, above, failed) = drive_chunk_self(85.6, (120.0, 0.05), (1200.0, 2.4, 1900.0), 60.0, secs);
        assert_eq!((above, failed), (0.0, 0), "차단점 위 {above}초 · 저장 거부 {failed} · 발화 {:?}", &fires[..fires.len().min(10)]);
        assert!(cycles >= 30, "사이클 {cycles} — 가장자리 짝 좌석이 최소 간격마다 clear 되지 않았다 · 발화 {:?}", &fires[..fires.len().min(10)]);
        let gaps: Vec<f64> = fires.windows(2).map(|w| w[1].0 - w[0].0).collect();
        assert!(gaps.iter().all(|g| *g >= CTX_GUARD_MIN_SPACING_SECS - 1e-6), "I1 최소 간격 위반 {gaps:?}");
        for (i, (a, _)) in fires.iter().enumerate() {
            assert!(fires[i..].iter().take_while(|(b, _)| *b - a < 3600.0).count() <= 6, "I1 굴림 1시간 > 6");
        }
    }

    // ───────────── ③ 성질 검체(무작위 사건열 1만 씨앗 · 결정론) ─────────────

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn f(&mut self) -> f64 {
            (self.next() % 1_000_000) as f64 / 1_000_000.0
        }
        fn pick(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[derive(Debug, Default)]
    struct PropStats {
        seeds: u64,
        fires: usize,
        min_spacing: f64,
        max_hour: usize,
        v_await: usize,
        v_cyc: usize,
        v_meas: usize,
        v_hold: usize,
        v_retry: usize,
        v_bar: usize,
        retries_ok: usize,
        v_i3: usize,
    }

    /// 참조 구현 prop.rs 이식 — 두 축 관측 · 범위 교대(헬퍼·다른 파일) · 낡은 자기보고 되풀이 · 압축 낙폭 · 사이클 표지(발화 뒤
    /// 지연·실패·늦음·외부) · 배달 동결 · 창 전환 · 틱. 오라클은 가드의 창 시작·끝만 받고 값은 따로 센다(차분).
    fn prop_run(seeds: u64) -> PropStats {
        let mut st = PropStats { seeds, min_spacing: f64::INFINITY, ..Default::default() };
        for seed in 1..=seeds {
            let mut r = Rng(0x9E3779B97F4A7C15 ^ seed.wrapping_mul(0x2545F4914F6CDD1D));
            let mut g = ClearGuard::default();
            let mut t = 0.0f64;
            let mut x: f64 = 20.0 + r.f() * 60.0;
            let mut window = match r.pick(4) {
                0 => None,
                1 => Some(1_000_000u64),
                _ => Some(200_000u64),
            };
            let base = [60u8, 60, 70, 80][r.pick(4) as usize];
            let rate = [0.0, 0.05, 0.2, 0.5, 1.0, 3.0][r.pick(6) as usize] / 60.0;
            let self_mode = r.pick(3);
            let sr_stale: Option<u8> = if r.pick(3) == 0 { Some(40 + r.pick(50) as u8) } else { None };
            let mut sess = 0u32;
            let mut fires: Vec<f64> = vec![];
            // (시각, 켬, 끔의 결과) — 끔은 clear 확인·모름·clear 안 됨(표지 켬 뒤 실패 · RR1-ROLE-1)이 섞인다.
            let mut sched: Vec<(f64, bool, cg::Outcome)> = vec![];
            let outcome_of = |k: u64| match k {
                0 => cg::Outcome::NotCleared,
                1 => cg::Outcome::Unknown,
                _ => cg::Outcome::Cleared,
            };
            let mut frozen_until = -1.0f64;
            let mut await_since: Option<f64> = None;
            let mut last_frozen = -1e9f64;
            let mut held_pending: Option<f64> = None;
            // 오라클 창: (시작, 끝, 축별 확인된 관측 최고치, 열림, 창) + 축별 (최고치를 낸 범위 · 확인 전 S 이상 외톨이(값, 범위)) +
            // 창 종류(사이클 뒤 창만 S 이상 외톨이 규칙). 명세(설계 §2 규칙 4 · R1V3-1): R = 창 안 관측 최고치 — 확인 전 S 이상
            // 외톨이만 뺀다(같은 범위의 최고치가 창에 있거나 같은 범위의 두 번째 S 이상 보고면 확인 · 둘 중 낮은 값).
            let mut win: (f64, f64, [Option<u8>; 2], bool, Option<u64>) = (0.0, 0.0, [None; 2], false, None);
            let mut win_scope: [Option<String>; 2] = [None, None];
            let mut win_hi: [Option<(u8, String)>; 2] = [None, None];
            let mut win_after_cycle = false;
            let mut m_missing = false;
            let horizon = 6.0 * 3600.0;
            while t < horizon {
                let dt = 1.0 + r.f() * 20.0;
                let prev_t = t;
                t += dt;
                let frozen = t < frozen_until;
                if frozen {
                    last_frozen = t;
                }
                x += rate * dt;
                if r.pick(60) == 0 {
                    x += r.f() * 8.0;
                }
                if (x >= 83.5 && r.pick(3) == 0) || r.pick(3000) == 0 {
                    x = 30.0 + r.f() * 40.0;
                }
                x = x.clamp(0.0, 100.0);
                if r.pick(6000) == 0 {
                    window = if window == Some(1_000_000) { Some(200_000) } else { Some(1_000_000) };
                }
                if r.pick(4000) == 0 {
                    frozen_until = t + 600.0 + r.f() * 7200.0;
                }
                if r.pick(5000) == 0 {
                    sched.push((t + r.f() * 60.0, true, cg::Outcome::Unknown));
                    sched.push((t + 60.0 + r.f() * 300.0, false, outcome_of(r.pick(5))));
                }
                if r.pick(300) == 0 {
                    m_missing = !m_missing;
                }
                sched.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                while let Some(&(at, on, outcome)) = sched.first() {
                    if at > t {
                        break;
                    }
                    sched.remove(0);
                    if on {
                        let _ = g.cycle_on(at.max(prev_t), frozen);
                    } else {
                        let _ = g.cycle_off(outcome, at.max(prev_t), frozen);
                        if outcome != cg::Outcome::NotCleared {
                            sess += 1;
                            x = 30.0 + r.f() * 60.0;
                        }
                    }
                }
                let o = g.tick(t, frozen);
                let fired_tick = matches!(o.verdict, Some(Verdict::Fire { .. }));
                // 보류 재판정 발화도 I3 — 판정한 관측(그 축의 가장 최근 관측)이 잰 수준 기준 막대 이상이다.
                if let Some(Verdict::Fire { pct: fp, axis: fa, window: fw, .. }) = o.verdict {
                    if let (Some(rp0), false) = (win.2[fa as usize], win.3) {
                        let rp = cg::rebase((rp0, win.4), fw);
                        let edge = stop_cap(fw).max(rp.saturating_add(1));
                        let need = rp.saturating_add(CTX_GUARD_GROWTH).min(edge).min(block_cap(fw)).max(base);
                        if fp < need {
                            st.v_i3 += 1;
                        }
                    }
                }
                if let Some(h) = held_pending {
                    if fired_tick {
                        st.retries_ok += 1;
                        held_pending = None;
                    } else if t > h + 25.0 && g.phase == Phase::Free {
                        st.v_retry += 1;
                        held_pending = None;
                    }
                }
                let mut fire_now = fired_tick;
                if !fire_now && r.pick(3) != 0 {
                    let (axis, scope, pct) = if self_mode == 1 || (self_mode == 2 && m_missing) {
                        (Axis::SelfReport, String::new(), sr_stale.unwrap_or(x.round() as u8))
                    } else if r.pick(12) == 0 {
                        (Axis::Measured, format!("h{}", r.pick(3)), (5 + r.pick(40)) as u8)
                    } else {
                        (Axis::Measured, format!("s{sess}"), x.round().clamp(0.0, 100.0) as u8)
                    };
                    // 오라클은 가드가 그 창으로 받는 관측만 센다 — 보고 직전 가드가 이 창(같은 기준 시각)을 재는 중이어야 한다(Cycling
                    // 중 관측은 수준이 아니다 · clear 안 됨 끔이 되돌린 창도 그 사이 관측은 받지 않았다).
                    let in_window = matches!(g.phase, Phase::Measuring { anchor, .. } if (anchor - win.0).abs() < 1e-9);
                    let o = g.report(&Rep { pct, window, axis, scope: &scope, base, now: t, frozen });
                    held_pending = None;
                    if win.3 && in_window && t <= win.1 {
                        if win.4 != window {
                            for k in 0..2 {
                                win.2[k] = win.2[k].map(|p| cg::rebase((p, win.4), window));
                                win_hi[k] = win_hi[k].take().map(|(p, sc)| (cg::rebase((p, win.4), window), sc));
                            }
                            win.4 = window;
                        }
                        let i = axis as usize;
                        // 확인 규칙(명세 · 설계 §2 규칙 4 · RNC5-1 · 수정 6회차 V42NC-1) — 사이클 뒤 창의 S 이상 보고: 같은 범위의
                        // 최고치가 창에 있거나 같은 범위의 두 번째 S 이상 보고(외톨이 짝)면 창을 닫는 가장자리 보고(수준 밖 — 곧바로
                        // 판정 · 짝의 낮은 값도 수준에 들지 않는다) · 그 밖은 외톨이(수준 밖). 그 밖의 창·S 미만 보고는 그대로 든다.
                        let counted = if win_after_cycle && pct >= stop_cap(window) {
                            if win_scope[i].as_deref() == Some(scope.as_str()) {
                                None
                            } else if win_hi[i].as_ref().is_some_and(|(_, hs)| *hs == scope) {
                                None
                            } else {
                                win_hi[i] = Some((pct, scope.clone()));
                                None
                            }
                        } else {
                            if win_hi[i].as_ref().is_some_and(|(_, hs)| *hs == scope) {
                                win_hi[i] = None;
                            }
                            Some(pct)
                        };
                        if let Some(v) = counted {
                            if win.2[i].is_none_or(|p| v > p) {
                                win_scope[i] = Some(scope.clone());
                            }
                            win.2[i] = Some(win.2[i].map_or(v, |p| p.max(v)));
                        }
                    }
                    if o.notes.iter().any(|n| matches!(n, Note::Measured { .. })) {
                        win.3 = false;
                    }
                    match o.verdict {
                        Some(Verdict::Held { until, .. }) => {
                            if until - t > CTX_GUARD_BACKOFF_MAX_SECS + 1e-6 {
                                st.v_hold += 1;
                            }
                            // 보류 재판정은 그 축의 가장 최근 관측을 판정한다(RR2-ROLE-1) — 이 보고가 그것일 때만 만료 발화를 기대한다.
                            if axis == Axis::Measured
                                && until - t < 3000.0
                                && g.retry_obs(Axis::Measured) == Some((cg::scope_id(&scope), pct))
                            {
                                held_pending = Some(until);
                            }
                        }
                        Some(Verdict::Fire { pct: fp, .. }) => {
                            // 발화는 창이 닫힌 뒤다(S 가장자리 보고는 제 창을 닫고 곧바로 판정된다 — 그 보고는 R 밖).
                            if let Some(rp0) = win.2[axis as usize] {
                                let rp = cg::rebase((rp0, win.4), window);
                                if !win.3 {
                                    let edge = stop_cap(window).max(rp.saturating_add(1));
                                    let need = rp.saturating_add(CTX_GUARD_GROWTH).min(edge).min(block_cap(window)).max(base);
                                    if fp < need {
                                        st.v_i3 += 1;
                                    }
                                }
                            }
                            fire_now = true;
                        }
                        _ => {}
                    }
                }
                if let Phase::Measuring { anchor, secs, kind, .. } = g.phase {
                    if !win.3 || (win.0 - anchor).abs() > 1e-9 {
                        win = (anchor, anchor + secs, [None; 2], true, window);
                        win_scope = [None, None];
                        win_hi = [None, None];
                        win_after_cycle = kind == Kind::AfterCycle;
                    }
                } else if win.3 {
                    win.3 = false;
                }
                if fire_now {
                    st.fires += 1;
                    if let Some(prev) = fires.last() {
                        st.min_spacing = st.min_spacing.min(t - prev);
                    }
                    fires.push(t);
                    if r.pick(8) != 0 {
                        let d1 = 20.0 + r.f() * 1500.0;
                        let d2 = 30.0 + r.f() * 400.0;
                        sched.push((t + d1, true, cg::Outcome::Unknown));
                        sched.push((t + d1 + d2, false, outcome_of(r.pick(5))));
                    }
                }
                match g.phase {
                    Phase::Awaiting { at, .. } => {
                        let from = *await_since.get_or_insert(at);
                        if t - from.max(last_frozen) > CTX_GUARD_CLEAR_WAIT_SECS + 26.0 {
                            st.v_await += 1;
                        }
                    }
                    Phase::Cycling { since } => {
                        await_since = None;
                        if t - since > CTX_GUARD_CYCLING_MAX_SECS + 25.0 {
                            st.v_cyc += 1;
                        }
                    }
                    Phase::Measuring { anchor, secs, .. } => {
                        await_since = None;
                        if t > anchor + secs + 25.0 {
                            st.v_meas += 1;
                        }
                    }
                    Phase::Free => await_since = None,
                }
                for a in [Axis::Measured, Axis::SelfReport] {
                    if g.bar(a, base, window) > base.max(block_cap(window)) {
                        st.v_bar += 1;
                    }
                }
            }
            for (i, a) in fires.iter().enumerate() {
                st.max_hour = st.max_hour.max(fires[i..].iter().take_while(|b| **b - a < 3600.0).count());
            }
        }
        st
    }

    fn prop_stats() -> &'static PropStats {
        static STATS: std::sync::OnceLock<PropStats> = std::sync::OnceLock::new();
        STATS.get_or_init(|| prop_run(10_000))
    }

    /// I1 — 무작위 사건열 1만 씨앗에서 연속 두 발화 간격 ≥ 600초 · 굴림 1시간 ≤ 6.
    #[test]
    fn i1_min_spacing_property() {
        let st = prop_stats();
        assert!(st.fires > 10_000, "사건열이 발화를 충분히 만들지 않았다(검사가 공허): {st:?}");
        assert!(st.min_spacing >= CTX_GUARD_MIN_SPACING_SECS - 1e-6 && st.max_hour <= 6, "{st:?}");
    }

    /// I2 — 비-Free 체류 상한(Awaiting ≤ 1200 + 동결 · Cycling ≤ 660 · Measuring ≤ 창) · 보류 ≤ 7200 · 보류된 실측 관측은 새
    /// 관측이 없어도 보류 만료 뒤 첫 틱에 발화 · 막대 ≤ max(기본, C).
    #[test]
    fn i2_bounded_latency_property() {
        let st = prop_stats();
        assert!(st.retries_ok > 100, "보류 재판정 발화가 거의 없다(검사가 공허): {st:?}");
        assert_eq!((st.v_await, st.v_cyc, st.v_meas, st.v_hold, st.v_retry, st.v_bar), (0, 0, 0, 0, 0, 0), "{st:?}");
    }

    /// I3 — 발화 퍼센트 ≥ 그 축의 '마지막 사이클 끝·압축 뒤 창 안 S 미만 관측 최고치'(오라클이 따로 센다) 기준 막대.
    #[test]
    fn i3_fire_implies_growth_property() {
        let st = prop_stats();
        assert_eq!(st.v_i3, 0, "{st:?}");
        assert_eq!(st.seeds, 10_000);
    }
}
