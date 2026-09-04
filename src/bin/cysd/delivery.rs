//! delivery — **배달 원장(delivery ledger)**: 데몬이 pane stdin 에 밀어 넣은 텍스트를
//! 주입 **직전에** 영속 기록하는 out-of-band 채널 (2026-08-01 R1 구조 수리).
//!
//! ## 왜 필요한가 (사고 기제)
//! 임무 게이트(`javis_mission.py`)는 "이 프롬프트를 오너가 쳤는가, 기계가 밀어 넣었는가"를
//! 갈라야 하는데, 훅은 둘을 **같은 자리(UserPromptSubmit)** 에서 본다. 종전 판별 근거는
//! 문자열 라벨 정규식 하나(`_MACHINE_LABEL`)뿐이었고, 그래서
//!   · 라벨 없는 push (`cys send --to master "다음 액션 착수"` — CEO_TEMPLATE.md:22 등 출하 규약)
//!   · 라벨 규약 우회 5종(중첩 대괄호·라벨 내 개행·80자 초과·선두 비공백·전각 대괄호)
//! 이 전부 **오너 임무**로 기록돼 자율주행 게이트를 열었다(= 사고 그 자체).
//!
//! 라벨 문자열은 **발신자가 고르는 값**이라, 규약을 안 지킨 push 하나로 게이트가 열렸다. 그래서
//! 판별 근거를 문자열에서 **주입한 쪽이 남긴 사실 기록**으로 옮긴다: 데몬이 주입 직전에 쓰는
//! 배달 원장. 훅은 프롬프트를 같은 규칙으로 해시해 원장과 대조하고, 일치하면 "기계가 방금 이
//! pane 에 밀어 넣은 바로 그 문자열" = 오너 임무 아님으로 접는다.
//! ★이것이 닫는 것은 **평시 정상 동작 경로**뿐이다 — 동일 UID 의 의도적 위조는 닫지 못한다.
//!   보장 범위의 정의처(SOT)는 `docs/THREAT-MODEL-mission-gate.md` 하나이며, 아래 '막을 수 없는
//!   것'은 그 문서의 배달층 각주다(모델 서술을 여기서 늘리지 말고 SOT 에 쓴다).
//!
//! ## 절대 불변식 3개 (튜닝 시 반드시 보존)
//! ① **원장 기록이 주입보다 반드시 선행한다.** 구현 규약: 기록을 `write_tx.try_send(..)`
//!    **직전**에 둔다. writer 스레드는 채널 수신 이후에만 PTY 에 쓰므로, 기록→try_send→(수신)→write
//!    순서가 구조적으로 보장된다(happens-before). 반대로 두면 훅이 "아직 없는 원장"을 읽어
//!    기계 push 를 오너 임무로 기록하는 race 가 열린다.
//! ② **데몬이 검증한 오너 GUI 입력만 기록에서 제외한다.** 오너 문장이 자기 해시와 매치돼
//!    기계로 접히면 온보딩이 전면 사망한다(임무를 영영 줄 수 없다). 단 제외 근거는 **클라이언트
//!    자기신고가 아니라 데몬이 아는 사실**이어야 한다 — 아래 R4 참조.
//! ③ **거짓 양성(기계→오너 오인)이 치명 / 거짓 음성(오너→기계 오인)은 경미**. 원장이 넓을수록
//!    안전하다 — 애매하면 기록한다(②의 제외는 **검증됐을 때만** 적용).
//!
//! ## ★R4 수리 — 기록 억제 근거를 '자기신고 human' → '데몬 검증 오퍼레이터'로 교체
//! 종전 `handlers.rs` 는 `if !human { record(..) }` 였다. 그런데 `human` 은 **클라이언트가
//! 스스로 붙이는 불리언**이고, 같은 함수의 ACL 주석이 이미 "어떤 pane 이든 위조 가능"이라
//! 못 박고 있었다 — 즉 **같은 코드가 ACL 목적으로는 human 을 불신하면서 원장 기록 여부만
//! 신뢰하는 비대칭**이 결함의 본체였다. 원시 소켓 한 줄
//! (`{"method":"surface.send_text","params":{...,"human":true}}`)로 원장 무기록 → 훅이 층2
//! 라벨 폴백 → 무라벨이라 통과 → 임무 게이트 개방이 실측됐다(라운드3 검증자 N3).
//!
//! 이제 억제 근거는 **데몬이 기동 시 자기가 발급하고 0600 으로 보관하는 `operator.token`** 이다
//! (`state.rs::write_operator_token` · GUI 승인 채널이 이미 쓰는 그 메커니즘). GUI(Tauri 백엔드)
//! 만이 그 파일을 읽어 첨부하고, 공용 `cys` CLI 는 **어떤 경로에서도 첨부하지 않는다**.
//!
//! ### 두 요구가 충돌하는 지점과 어느 쪽으로 접었는지 (명시)
//!  · 요구 A(불변식 ②): 진짜 오너 GUI 키 입력은 기록되면 **안 된다**.
//!  · 요구 B(fail-closed): 발신 주체를 데몬이 확정 못 하면 **기록해야** 한다(기계로 취급).
//!  두 요구는 "GUI 인지 아닌지 판정 불가"인 지점에서 충돌한다. **B 로 접었다** — 판정 불가는
//!  기록한다. 반대로 B 를 포기하면 원시 소켓 한 줄로 게이트가 열린다(실측된 치명 결함).
//!  그래서 GUI 는 토큰을 첨부하도록 배선하고(`src-tauri/src/main.rs::send_input`), 토큰이 없거나
//!  틀리면 **기록**한다.
//!
//! ## ★★R5 수리 — `operator_token` 은 '사람이 앉은 GUI 세션'이지 '사람이 친 문장'이 아니다
//! 라운드4 검증자 실측(신규 치명): GUI 는 **사용자가 자판으로 친 입력**만 보내는 게 아니라
//! **자기가 만든 문안**도 같은 `surface.send_text` 로 보낸다 — 전출 지시 전문(`ui/src/main.ts`
//! `clear_first:true` + 자동 CR)·노드 재기동 명령·경로 삽입이 그것이다. R4 배선은 그 호출에도
//! 토큰을 붙였으므로 `human_verified=true` 가 되어 **원장에 아무것도 남지 않았고**, 훅은 층2
//! 라벨 폴백으로 내려가 무라벨 문안을 **오너 임무로 기록**했다(실측 rc=0 · 흔적 0).
//!
//! 근본 교정: 토큰은 "이 요청이 오퍼레이터 GUI 세션에서 왔다"만 증명한다. 거기에 더해 UI 가
//! **그 문안을 사람이 쳤는지, UI 코드가 만들었는지**를 알려야 한다. 그래서 신호를 하나 더 흘린다
//! (`machine_origin` · UI → tauri → cysd). **표식이 있으면 토큰이 유효해도 기록**하며 원장
//! `origin` 은 `gui_auto` 로 남아 감사에서 구별된다.
//!
//! ★불변식 ② 절대 보존 — 두 경로를 코드에서 명확히 가른다:
//!   · `ui/src/main.ts::sendRaw`(= `term.onData`/붙여넣기 = **사람이 친 실키**) → 표식 **없음**
//!     → 종전대로 무기록. 여기가 기록되면 오너가 임무를 줄 수 없어 온보딩이 전면 사망한다.
//!   · UI 가 문자열을 조립해 보내는 호출(전출 지시·`launchCmd`·`restartNode`·`injectRawToPane`)
//!     → 표식 **있음** → 기록. 새 자동 주입을 UI 에 추가할 때 표식을 빠뜨리면 이 구멍이 재발하므로,
//!     "UI 가 만든 문자열을 보내면 표식"을 규칙으로 삼는다(사람 키만 예외).
//!
//! ★정직 고지: 이 표식도 **클라이언트가 붙이는 값**이다. 위조 방향은 "표식을 떼는 것"인데,
//!   표식을 떼도 **토큰까지 있어야** 기록이 억제되므로 공격자는 결국 토큰 파일을 읽어야 한다 —
//!   그 순간 §2 OUT OF SCOPE(동일 UID 의도적 위조)이고 `delivery.operator_token_from_pane`
//!   감사 이벤트의 대상이다. 즉 이 표식이 닫는 것은 **평시 정상 동작**(GUI 가 선의로 만든 자동
//!   주입)이 게이트를 여는 경로뿐이다.
//!
//! ## ★막을 수 없는 것 (정직 고지 — 닫는 시늉 금지 · 전문은 SOT)
//! 정의처는 `docs/THREAT-MODEL-mission-gate.md` §2 다. 여기 적는 것은 **배달층에 국한된 각주**이며,
//! 범위·근거·감사 절차를 늘려 쓰고 싶으면 SOT 를 고친다(두 곳에 쓰면 반드시 어긋난다).
//! 이 원장은 **동일 UID 구성요소의 의도적 위조를 막지 못한다.** master 는 Bash 를 가지므로
//! ① `operator.token` 파일을 그대로 읽어 첨부할 수 있고 ② 원장 파일을 직접 지우거나 덮어쓸 수
//! 있고 ③ 임무 대장을 직접 쓸 수 있다. 어떤 in-band 장치로도 원리적으로 닫히지 않는다
//! (동일 신뢰경계 안에서의 자기위조 = 커널 밖에서 풀 수 없는 문제).
//! 이 층이 실제로 보장하는 것은 **평시 정상 동작이 자율 착수 권한을 발급하지 않는다** 하나다.
//! 위조 범주에 대한 무기는 차단이 아니라 **감사 가능성**이다 — 기록 실패·판독 실패·창 밖 배달·
//! pane 에서 온 오퍼레이터 토큰을 전부 사유 코드로 남기고(`delivery.*` 이벤트 + 임무 대장
//! `anomalies`), master 가 그것을 오너에게 보고하게 한다. "완전 방어"를 주장하는 문장은
//! 그 자체가 결함이다.
//!
//! ## 경로 계약 (★두 디렉터리를 혼동하지 말 것)
//! Rust `state::state_dir(socket)` = 소켓 옆(`~/.local/state/cys`) 이고, **팩 계약**은
//! `CYS_STATE_DIR ‖ ~/.cys/state` 다(`javis_bootstrap.state_dir()`). 훅이 읽는 쪽은 후자이므로
//! 원장은 **팩 계약 경로**에 쓴다. 파일명은 항상 레인 접미(`delivery-<lane>.jsonl`) —
//! 부서 레인의 배달이 base master 의 판별에 섞이면 안 된다(`javis_bootstrap.lane_state_path`
//! 의 skip·lock 과 동일 규약).
//!
//! ## ★★R6 수리 — 원장의 기록 단위를 **실제 제출 단위**에 맞춘다 (멀티라인 행 분할)
//! 라운드5 검증자 실측(관통·치명): `record` 는 전문 1건만 남기는데, 비-`clear_first` 경로는
//! `WriteReq::Program(text.as_bytes())`(0.14.24 — 종전 `Data`·바이트 동일) 로 **원시 바이트**를 PTY 에 쓴다(`handlers.rs`). 본문의
//! 개행은 그대로 Enter 가 되므로 TUI 는 그것을 **행 단위로 쪼개 여러 번 제출**한다. 그러면
//! 제출된 각 프롬프트는 원장 레코드의 **진부분**이라 ⓐ전문 해시가 어긋나고 ⓑ판독자의 부분
//! 일치도 `chars > len(prompt)` 조건에서 레코드를 통째로 건너뛴다 → 층1 전건 미스 → 무라벨이면
//! 층2도 통과 → 게이트 개방.
//!
//! 근본 교정은 **원장이 '실제로 무엇이 제출되는가'를 반영하는 것**이다(층1 의 전제). 그래서
//! `record` 는 전문 레코드에 더해 **제출 단위(개행 분할) 조각**을 각각 한 줄씩 남긴다
//! (`part`·`parent` 필드 · 전문 레코드에는 `units` = 조각 수). 판정 규칙은 하나도 늘지 않는다 —
//! 조각도 그냥 레코드이므로 판독자는 **종전의 전문 해시 대조**로 잡는다(구 판독자도 그대로 잡는다
//! = 팩 스큐 순방향 호환).
//!
//! ★대안 비교(택하지 않은 것): "Data 경로도 bracketed paste 로 감싸 분할 자체를 없앤다."
//!   실제 분할을 없애는 가장 근본적인 수리지만 **주입 런타임 동작을 바꾼다** — bracketed paste 를
//!   켜지 않은 수신 앱(평범한 셸·구 TUI)에는 `ESC[200~` 가 그대로 문자로 들어가 배달이 깨지고,
//!   셸 pane 에서는 "여러 줄을 즉시 실행" 의미가 사라진다. 라이브에서 검증할 수 없는 변경이라
//!   **관측(원장)을 현실에 맞추는 쪽**을 택했다. (`Inject` 분기는 이미 bracketed paste 라 한 덩어리로
//!   제출되지만, 앱이 그 모드를 안 켰으면 거기서도 쪼개진다 — 그래서 조각은 **경로를 가리지 않고**
//!   남긴다. 불변식 ③: 애매하면 기록한다.)
//!
//! ## 판독자(python) 와의 계약
//! `javis_mission.py` 가 같은 정규화·같은 해시·같은 경로 규약을 구현한다. 양쪽 규칙이 갈리면
//! 원장은 조용히 무력화되므로, 정규화 규칙은 이 파일과 `javis_mission._normalize_delivery`
//! 주석에 **동일 문구로** 박제하고 양쪽에 회귀 테스트를 둔다.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 원장 레코드 스키마 버전. 판독자(javis_mission)가 미지 버전을 만나면 '판독 불가'로 접는다.
pub const LEDGER_SCHEMA: u64 = 1;

/// 원장 파일 크기 상한(바이트). 초과 시 1세대 회전(`.1` 로 rename 후 새 파일).
///
/// ★R7 상향 2 MiB → 8 MiB (실측 근거 · SOT §4-6). R6 이 조각 레코드를 도입하면서 **디렉티브급
/// push 1회의 원장 비용이 294 B → 약 220 KB** 가 됐고, 2 MiB 에서는 그런 push 10 회면 회전해
/// 과거 구간의 층1 대조 근거가 통째로 사라졌다(회전 꼬리 = SOT §4-6 ⓑ '치명 방향').
///
/// 상한을 무한정 올릴 수는 없다 — 판독자(`javis_mission.read_delivery`)는 **오너 프롬프트마다**
/// 본 세대 + 회전 세대를 전수 파싱하므로 비용이 원장 크기에 선형이다. 실측(python 3.13 ·
/// 이 저장소 디렉티브 실문안): 판독+판정 **약 8.3 ms/MiB**.
///   · 2 MiB(현행) → 2세대 4 MiB → 33 ms
///   · 8 MiB(채택) → 2세대 16 MiB → 135 ms   ← 대화형 훅이 감당하는 상한으로 본다
///   · 16 MiB      → 2세대 32 MiB → 267 ms   ← 매 프롬프트 지연으로 과하다(기각)
/// 즉 이 값의 상한을 정하는 것은 디스크가 아니라 **훅 지연**이다. 바꿀 때는 반드시
/// `javis_mission.LEDGER_MAX_READ_BYTES`(= 이 값의 2배)와 `DELIVERY_SCAN_LINES`(아래 주석의
/// 유도식)를 함께 옮긴다 — 한쪽만 올리면 정상 데몬 출력이 판독자의 '조작 정황' 상한을 넘겨
/// **게이트가 영구 fail-closed** 로 잠긴다(부트스트랩 불가침 위반).
pub const LEDGER_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 레코드에 남기는 정규화 본문 미리보기 상한(문자). 판정은 sha256 로 하며 이 값은 **진단용**이자
/// 판독자의 **부분 일치 앵커**다(SOT §3). 원장에 본문 전체를 남기면 그 자체가 프롬프트 유출
/// 저장소가 된다 — 짧게 자른다.
pub const PREVIEW_CHARS: usize = 64;

/// **조각 레코드**의 미리보기 상한(문자) — 전문 레코드(`PREVIEW_CHARS`)보다 짧다(R7 경량화).
///
/// 앵커로서의 요건은 "정규화 본문의 **접두사**일 것" 하나다(판독자는 `find(preview)` 로 후보
/// 위치를 잡고 `chars` 길이를 잘라 sha256 로 **확증**한다). 짧아지면 후보 위치가 늘어 탐색
/// 예산(`DELIVERY_SPAN_OCC_BUDGET` 10만)을 조금 더 쓸 뿐 **판정은 한 글자도 바뀌지 않는다**.
/// 24자로 줄이면 한글 조각 1건이 약 417 B → 318 B(실측 -24%)가 되고, 그만큼 회전이 늦어진다.
/// (`DELIVERY_PART_MIN_CHARS` 와 같은 24 인 것은 우연이 아니다 — 그 하한보다 짧은 앵커는
/// 부분 일치 규칙이 어차피 단독으로 쓰지 않는다.)
pub const PART_PREVIEW_CHARS: usize = 24;

/// 배달 1건이 남기는 **제출 단위 조각**(개행 분할) 레코드의 최대 개수(R6 · R7 상향).
///
/// 상한을 두는 이유는 성능이 아니라 **원장 예산**이다 — 조각이 무제한이면 긴 지침 1회 주입이
/// 크기 상한을 밀어 회전시켜 과거 배달의 판별 근거를 통째로 날린다(회전 꼬리 = SOT §4-6 ⓑ).
///
/// ★R7 상향 500 → 4000 (실측 근거 · SOT §4-7 ⓑ). 라운드6 은 "실배포 디렉티브 최대 454 단위 ⇒
/// 여유 46" 으로 봤지만 그건 **파일 하나**의 값이다. 실제로 pane 에 들어가는 것은
/// `cys.rs::compose_directive` 가 합성한 문안(역할 디렉티브 + RSI + soul.md + 장기메모리 색인 +
/// 스킬 색인 + 오버레이)이고, 실측하면 master 가 **699 고유 제출단위**다 — 즉 500 상한은
/// **이미 초과 상태**였고 매 `launch-agent`·`reinject`·`cycle` 마다 약 200 행이 원장에서
/// 조용히 빠지고 있었다(회귀 핀: `deployed_directive_payload_fits_part_cap_with_headroom`).
/// 4000 은 실측 최대(699)의 5.7 배이며, 초과분 1건의 원장 비용은 4000 × ~297 B ≈ 1.2 MB 로
/// 위 `LEDGER_MAX_BYTES` 의 14% 다(한 번의 초장문 push 가 원장을 통째로 밀어내지 못한다).
/// ★수치의 정의처는 SOT `docs/THREAT-MODEL-mission-gate.md` §4-6·§4-7 이다 — 여기 값이 그
/// 문서와 갈리면 그 문서가 맞다(0.14.10 검증에서 702·318 B 가 stale 이어서 정정했다).
///
/// 초과분은 조용히 버리지 않는다 — ⓐ`delivery.parts_incomplete` 이벤트(데몬 버스)와
/// ⓑ**전문 레코드의 `parts_capped` 필드**로 남긴다. ⓑ 가 R7 의 본체다: 데몬 버스는 임무
/// 게이트의 판독 경로가 아니어서, 종전에는 초과가 나도 임무 verdict 에 흔적이 0 이었다.
/// 판독자는 `parts_capped` 를 보면 **그 배달에 대한 판정을 접는다**(fail-closed · 불변식 ③ ·
/// `javis_mission.py::DELIVERY_CAPPED_FOLD_S`).
pub const MAX_PARTS: usize = 4000;

/// 원장 append 1회 write 의 바이트 예산. O_APPEND 단일 write 는 이 크기 이하에서 사실상 원자라
/// 여러 스레드가 붙어도 줄이 섞이지 않는다 — 조각 N 건을 **한 번에** 쓰되 이 크기로 끊는다
/// (열기/닫기는 1회, write 는 여러 번).
const APPEND_CHUNK_BYTES: usize = 4096;

/// 주입 유래 — 원장의 `origin` 필드. 판정에는 쓰이지 않고 사후 진단·감사에 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `cys send` 직접 배달(handlers::surface.send_text · Data/Inject 양 분기)
    Send,
    /// `--queued` 배달자(watchdog)가 조용해진 pane 에 밀어 넣는 순간
    Queue,
    /// schedule 잡 발화(schedule::inject)
    Schedule,
    /// 승인 Feed 의 CEO 자동 라우팅 주입
    Feed,
    /// 외부 채널(inbox) 봉투 주입
    Channel,
    /// 빈 좌석 승계 고지(`# [cys] …` 주석 주입)
    SeatTakeover,
    /// ★GUI 가 **프로그램적으로 만든** 주입(전출 지시·노드 재기동 명령·경로 삽입 등).
    /// 오퍼레이터 토큰이 붙어 있어도 **기록한다** — 사람이 앉은 세션이라는 사실과 사람이 친
    /// 문장이라는 사실은 다르다(아래 'R5 수리' 절).
    GuiAuto,
    /// ★기동 표식(sentinel) — 실제 배달이 아니다. 데몬이 기동 시 1줄 append 해
    /// "정상 원장은 절대 0바이트가 아니다"를 성립시킨다(판독자의 '빈 파일 = 손상' 근거).
    Boot,
    /// ★(U-23) **부트 감독자**(`boot_supervisor`)가 인텐트를 집행하기 직전에 남기는 표식.
    ///
    /// `Boot` 과 같은 성격이다 — pane 에 글자를 밀어 넣는 '배달'이 아니라 **기계 유래 근거**다.
    /// 왜 필요한가: 감독자가 낳은 부트 체인은 곧 pane 에 지침을 밀어 넣는다. 그 순간 원장에
    /// 아무 근거가 없으면 임무 게이트는 그 push 를 **오너 임무로 오인**하고 자율 착수 권한을
    /// 오발급한다. 그래서 감독자는 **행위보다 먼저** 이 유래로 기록한다
    /// (`boot_supervisor::dispatch_one` · 순서 불변식은 같은 파일의 소스 핀이 지킨다).
    Supervisor,
    /// ★환경 진단 고지(`# [cys] …` 주석 1줄 주입) — npm 번들 오염 경고가 첫 소비자다.
    ///
    /// `SeatTakeover` 와 같은 성격이다: 사용자에게 **보여 주려고** pane 에 밀어 넣는 기계
    /// 문장이므로 원장에 근거가 있어야 한다. 없으면 임무 게이트가 이 주석을 오너 임무로 읽는다.
    EnvAdvisory,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Send => "send",
            Origin::Queue => "queue",
            Origin::Schedule => "schedule",
            Origin::Feed => "feed",
            Origin::Channel => "channel",
            Origin::SeatTakeover => "seat_takeover",
            Origin::Boot => "boot",
            Origin::GuiAuto => "gui_auto",
            Origin::Supervisor => "supervisor",
            Origin::EnvAdvisory => "env_advisory",
        }
    }
}

/// `record` 의 결과 — 종전 `bool` 은 "공백이라 안 씀"과 "쓰려다 실패"를 같은 false 로 뭉쳐서
/// **기록 실패가 조용히 사라졌다**(그 상태에서 주입된 텍스트는 훅에 오너 임무로 보인다 = 게이트
/// 개방). 감사 가능성 확보를 위해 셋을 가른다.
#[derive(Debug)]
pub enum Outcome {
    /// 원장에 append 됐다(판별 근거 확보).
    Recorded,
    /// 정규화 후 빈 문자열 — 프롬프트가 될 수 없어 기록 대상이 아니다(정상).
    Blank,
    /// ★기록 **실패**. 이 주입은 원장에 없으므로 훅이 오너 임무로 오인할 수 있다.
    Failed(String),
}

/// ★정규화 규칙 (판독자 `javis_mission._normalize_delivery` 와 **문자 단위로 동일**해야 한다)
///   ① 모든 유니코드 공백(White_Space)을 ASCII 공백 하나로 치환
///   ② 연속 공백을 1개로 접는다
///   ③ 앞뒤 공백 제거
/// 대소문자·유니코드 정규화(NFC)는 **하지 않는다** — 접는 폭이 넓을수록 오너 문장이 기계로
/// 오인될 확률만 커지고(경미하지만 불필요), TUI 는 코드포인트를 바꾸지 않는다.
///
/// 공백 집합을 `char::is_whitespace()`(= Unicode White_Space property)로 정의한 이유:
/// python `re.compile(r"\s+")` 는 여기에 더해 U+001C..U+001F 를 포함해 **미세하게 다르다**.
/// 그래서 판독자도 라이브러리 의미에 기대지 않고 White_Space 집합을 명시 구현한다(양쪽 박제).
///
/// ★부트 v2(B1): 구현이 **lib `cys::mission_gate` 로 이관**됐고 여기는 위임만 남는다.
/// 이유: `cys hook`(CLI 바이너리)이 층1 원장 대조를 하려면 같은 산식이 필요한데, 종전에는 이
/// 함수가 `cysd` 바이너리 안에만 있어 사본을 만들 수밖에 없었다. 사본이 갈리는 순간 해시가
/// 안 맞아 원장 대조는 **조용히** 무력화된다(항상 '미일치' = fail-open). 이 파일의 기존
/// 검체들은 위임을 관통해 그대로 계약을 잰다 — 그것이 이관의 회귀 방어다.
pub fn normalize(text: &str) -> String {
    cys::mission_gate::normalize(text)
}

/// 이미 정규화된 문자열의 sha256 소문자 hex(lib 위임 — 위 `normalize` 주석 참조).
fn digest_normalized(norm: &str) -> String {
    cys::mission_gate::digest_normalized(norm)
}

/// 정규화 본문의 sha256 소문자 hex. **테스트 전용 편의 함수**다.
///
/// ★R4 정정: 라운드3 보고서는 "cargo check 신규 경고 0" 이라고 썼지만 사실이 아니었다 —
/// 이 함수가 `dead_code` 경고를 내고 있었다(`warning: function \`digest\` is never used`).
/// 생산 경로(`record`)는 `normalize` 결과를 preview·chars 산출에 재사용해야 해서
/// `normalize` + `digest_normalized` 를 따로 부른다(이중 정규화 회피). 즉 이 함수는 배선
/// 누락이 아니라 **교차언어 앵커용 테스트 헬퍼**이므로 test 빌드로 좁힌다(삭제하지 않는 이유:
/// python `javis_mission.delivery_digest` 와의 동치를 박제하는 것이 이 함수의 유일한 임무다).
#[cfg(test)]
pub fn digest(text: &str) -> String {
    digest_text(text)
}

/// ★B1(0.14.30): 원문 → 정규화 → sha256(원장 대조 키와 **같은 산식**).
///
/// 왜 생산 경로에 필요해졌나: 병합 배달(다이제스트)은 여러 항목을 한 본문으로 주입하므로
/// 전문 레코드의 sha 는 합성본의 것이다. 그러면 "보낸 항목이 전부 배달됐나(유실 0)" 를 원장만
/// 보고는 확인할 수 없다 — 수용 기준 §7 ①이 요구하는 sha 전수 대조가 성립하려면 항목별 sha 가
/// 원장에 있어야 한다. `digest_parts[].sha256` 이 그 자리다.
pub fn digest_text(text: &str) -> String {
    digest_normalized(&normalize(text))
}

/// 팩 계약 상태 루트 — `CYS_STATE_DIR` 우선, 없으면 `~/.cys/state`.
/// (`javis_bootstrap.state_dir()` 와 동일 규약 — 사본이 아니라 **미러**이며, 갈리면 원장이
/// 조용히 무력화되므로 양쪽에 테스트를 둔다.)
///
/// ★R5-B 테스트 위생: **테스트 빌드에서는 실 HOME 으로 해소되지 않는다**(`default_state_root`
/// 의 cfg 분기). 근거와 대안 비교는 아래 `tests` 모듈 머리말에 있다.
pub fn pack_state_dir() -> PathBuf {
    // 테스트 스레드 전용 격리 루트가 있으면 그것이 최우선(락 없이 테스트마다 분리된다).
    #[cfg(test)]
    if let Some(p) = tests::thread_state_root() {
        return p;
    }
    match std::env::var("CYS_STATE_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => default_state_root(),
    }
}

/// `CYS_STATE_DIR` 미설정 시의 기본 루트 — **생산 빌드**: 팩 계약 경로(`~/.cys/state`).
#[cfg(not(test))]
fn default_state_root() -> PathBuf {
    cys::home_dir().join(".cys").join("state")
}

/// `CYS_STATE_DIR` 미설정 시의 기본 루트 — **테스트 빌드**: 실 HOME 이 아니라 temp 샌드박스.
/// (개별 테스트가 격리를 잊어도 라이브 원장이 더러워지지 않게 하는 최종 방어선 · R5-B)
#[cfg(test)]
fn default_state_root() -> PathBuf {
    tests::unisolated_sandbox_root()
}

/// `javis_bootstrap._socket_is_base` 미러. 소켓 미설정('')=base.
fn socket_is_base(sock: &str) -> bool {
    let sock = sock.trim();
    if sock.is_empty() {
        return true;
    }
    let norm = sock.replace('\\', "/");
    if sock.starts_with("\\\\") || norm.to_ascii_lowercase().starts_with("//./pipe/") {
        // Windows named pipe — 성분 분해 부적합. 기존 basename 동작 보존.
        let last = norm.rsplit('/').next().unwrap_or("");
        return last == "cys" || last == "cys.sock";
    }
    for part in norm.split('/') {
        if part.starts_with("cys-dept-") {
            return false;
        }
    }
    // ★P0 수리(2026-09-04 실사고): 종전엔 **basename 만** 봤다 — 디렉터리를 무시했으므로
    //   `~/.cys/state-harness/cys.sock` · `/var/folders/…/l1-new-*/cys.sock` 처럼 **관례적
    //   파일명을 유지한 격리 소켓이 전부 base 로 접혔고**, 그 데몬들이 본 레인
    //   `delivery-base.jsonl`·`.epoch.json` 에 썼다(실측: 외부 좌석 레코드 70건 · epoch 덮어씀).
    //   귀결은 본부 임무 게이트 오탐 폐쇄 + 전 노드 층1 원장 오염이다.
    //
    //   역설이 이 결함의 핵심이었다: `/tmp/whatever.sock` 처럼 **파일명이 다르면** 올바로
    //   격리됐다. 즉 **가장 흔한 격리 방식**(관례 파일명 유지 + 디렉터리 분리)만 조용히 실패했다.
    //
    //   수리: 기본 소켓은 **`…/state/cys/cys.sock`** 이므로 **부모 디렉터리 이름까지** 본다.
    //   `cys` 디렉터리 안의 `cys.sock` 만 base 이고 나머지는 전부 자기 레인이다(fail-closed).
    //
    //   ★기계 독립 판정을 유지한다: 이 기계의 실제 홈 경로와 비교하지 않고 **경로 모양**만 본다.
    //   그래야 python 과 공유하는 소켓→lane_key 매트릭스 fixture 가 기계마다 같은 답을 낸다.
    //   하위호환: 진짜 기본 소켓은 여전히 base(기존 `delivery-base.jsonl` 유효) ·
    //   `cys-dept-`·`/tmp/whatever.sock` 은 종전대로 비-base. **바뀌는 것은 누출 경로 하나뿐이다.**
    let mut it = norm.rsplit('/');
    let last = it.next().unwrap_or("");
    if last != "cys" && last != "cys.sock" {
        return false;
    }
    // 부모 디렉터리가 정확히 `cys` 여야 한다(기본 소켓 `…/state/cys/cys.sock` 의 모양).
    it.next() == Some("cys")
}

/// SHA-1 hex — **비암호학적 용도 전용**(파일명 슬러그가 `javis_bootstrap._sanitize_sock_key`
/// (`hashlib.sha1(...).hexdigest()[:16]`)와 **정확히** 같아야 하기 때문에 존재한다).
/// 새 크레이트를 들이지 않는다(오프라인 빌드 계약 · sha1 은 Cargo.lock 에 전이로도 없다).
/// 서명·인증에 쓰지 말 것 — 그쪽은 `sha2`/`minisign-verify` 가 소유한다.
/// 정확성은 표준 시험벡터(`sha1("abc")`)로 박제한다.
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

/// `javis_bootstrap._sanitize_sock_key` 미러(경로 구분자·':' → '_' · 과길면 앞 120자+sha1 16자).
fn sanitize_sock_key(sock: &str) -> String {
    let mut raw = sock.trim().to_string();
    if raw.is_empty() {
        raw = "base".to_string();
    }
    // python 은 os.sep, '/', '\\', ':' 를 치환한다. unix os.sep='/', windows os.sep='\\' 이므로
    // 셋의 합집합은 어느 OS 에서도 {'/','\\',':'} 로 동일하다.
    raw = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c })
        .collect();
    let raw = raw.trim_matches('_').to_string();
    let mut raw = if raw.is_empty() { "base".to_string() } else { raw };
    if raw.chars().count() > 160 {
        let head: String = raw.chars().take(120).collect();
        let h = sha1_hex(raw.as_bytes());
        raw = format!("{head}-{}", &h[..16]);
    }
    raw
}

/// 이 데몬이 속한 레인 키 — `javis_bootstrap.lane_key` 미러.
/// pane 은 `CYS_SOCKET`(= 이 데몬의 socket_path, state.rs 가 주입)을 보므로 양쪽 값이 일치한다.
pub fn lane_key(socket_path: &Path) -> String {
    let s = socket_path.to_string_lossy();
    if socket_is_base(&s) {
        "base".to_string()
    } else {
        sanitize_sock_key(&s)
    }
}

/// 배달 원장 경로 — **항상 레인 접미**(base 레인도 `delivery-base.jsonl`).
/// skip·lock 과 같은 규약이다: 역사적 무접미 경로가 없으므로 base 예외를 만들 이유가 없고,
/// 접미가 항상 있으면 "이 파일이 어느 레인 것인가"가 파일명만으로 결정론이다.
pub fn ledger_path(socket_path: &Path) -> PathBuf {
    pack_state_dir().join(format!("delivery-{}.jsonl", lane_key(socket_path)))
}

/// 데몬 인스턴스 표식 경로 — 임무의 **세션 결박**(過去 임무 무기한 유효 차단)에 쓴다.
pub fn epoch_path(socket_path: &Path) -> PathBuf {
    pack_state_dir().join(format!("delivery-{}.epoch.json", lane_key(socket_path)))
}

fn iso_utc(epoch: f64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(
        epoch as i64,
        ((epoch.fract().max(0.0)) * 1e9) as u32,
    )
    .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
    .unwrap_or_default()
}

/// 데몬 기동 시 1회 — 이 인스턴스의 표식을 쓴다(best-effort · 실패해도 기동을 막지 않는다).
///
/// 판독자 계약: `javis_mission.gate()` 는 임무 기록 시점의 `daemon_epoch` 를 대장에 박아 두고,
/// 판정 시 이 파일을 다시 읽어 **같은 데몬 인스턴스인지** 확인한다. 데몬이 재기동했으면
/// (= 사실상 새 세션) 과거 임무는 무효다. 파일이 없으면 결박은 degrade 되고 TTL 만 남는다.
pub fn write_epoch(socket_path: &Path) {
    let p = epoch_path(socket_path);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let epoch = crate::state::now_epoch();
    let rec = json!({
        "v": LEDGER_SCHEMA,
        "daemon_epoch": epoch,
        "started": iso_utc(epoch),
        "pid": std::process::id(),
        "socket": socket_path.to_string_lossy(),
    });
    // 원자 교체 — 판독자가 부분 쓰기를 만나 '판독 불가'로 접지 않게.
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, rec.to_string().as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// ★기동 시 1회 — 원장에 **기동 표식(sentinel)** 1줄을 append 한다(`write_epoch` 와 짝).
///
/// 왜 필요한가(R4 fail-open ② 봉합): 종전 판독자는 "파일이 존재하는데 0바이트"를 **정상**으로
/// 셌다(`bad=0, good=0` → LEDGER_OK). 그래서 원장을 `: > delivery-base.jsonl` 로 비우기만 하면
/// 대조할 해시가 사라져 모든 기계 push 가 층2 라벨 폴백으로 내려가고, 무라벨 push 는 그대로
/// 오너 임무가 됐다. 데몬이 기동 때마다 표식 1줄을 남기면 **정상 원장은 절대 0바이트가 아니다**
/// 가 성립하고, 판독자는 '빈 파일 = 손상'을 fail-closed 로 판정할 수 있다.
///
/// 표식 레코드는 **구 판독자에서도 정상 파싱돼야 한다**(팩 스큐: 신 데몬 + 구 javis_mission).
/// 그래서 `sha256`·`ts_epoch`·`surface` 필드를 전부 채우되, surface 는 어떤 pane 과도 매치되지
/// 않는 `"-"`(surface id 는 항상 정수 문자열)로 둔다 — 구 판독자는 '남의 pane 배달'로 건너뛰고
/// `good` 으로 세므로 손상 오판이 나지 않는다.
pub fn write_boot_sentinel(socket_path: &Path) -> Outcome {
    let p = ledger_path(socket_path);
    if let Some(d) = p.parent() {
        if let Err(e) = std::fs::create_dir_all(d) {
            return Outcome::Failed(format!("상태 디렉터리 생성 실패: {e}"));
        }
    }
    rotate_if_needed(&p);
    let epoch = crate::state::now_epoch();
    let rec = json!({
        "v": LEDGER_SCHEMA,
        "surface": "-",           // 어떤 pane 과도 매치되지 않는 표기(surface id 는 정수 문자열)
        "ts_epoch": epoch,
        "ts": iso_utc(epoch),
        "sha256": "-",            // 어떤 프롬프트의 sha256 과도 같을 수 없다
        "origin": Origin::Boot.as_str(),
        "from": Value::Null,
        "chars": 0,
        "preview": "",
        "daemon_epoch": epoch,
        "pid": std::process::id(),
    });
    append_line(&p, &rec)
}

/// 원장 파일에 JSON 1줄 append(공통부). append 모드 단일 write — O_APPEND 라 여러 스레드가
/// 붙어도 라인이 섞이지 않는다(PIPE_BUF 이하 · 레코드는 수백 바이트).
fn append_line(p: &Path, rec: &Value) -> Outcome {
    append_lines(p, std::slice::from_ref(rec))
}

/// 여러 레코드를 **한 번 열어** append 한다(R6 조각 기록용). 파일 열기는 1회지만 write 는
/// `APPEND_CHUNK_BYTES` 이하로 끊는다 — 한 번에 수십 KB 를 쓰면 O_APPEND 원자성이 깨져
/// 동시 기록자와 줄이 섞일 수 있기 때문이다(섞인 줄은 판독자에서 `ledger_bad_lines`).
fn append_lines(p: &Path, recs: &[Value]) -> Outcome {
    if recs.is_empty() {
        return Outcome::Recorded;
    }
    let mut f = match std::fs::OpenOptions::new().create(true).append(true).open(p) {
        Ok(f) => f,
        Err(e) => return Outcome::Failed(format!("원장 open 실패({}): {e}", p.display())),
    };
    let mut buf = String::new();
    for rec in recs {
        let mut line = rec.to_string();
        line.push('\n');
        if !buf.is_empty() && buf.len() + line.len() > APPEND_CHUNK_BYTES {
            if let Err(e) = f.write_all(buf.as_bytes()) {
                return Outcome::Failed(format!("원장 write 실패: {e}"));
            }
            buf.clear();
        }
        buf.push_str(&line);
    }
    match f.write_all(buf.as_bytes()).and_then(|_| f.flush()) {
        Ok(()) => Outcome::Recorded,
        Err(e) => Outcome::Failed(format!("원장 write 실패: {e}")),
    }
}

/// ★R6 — 이 텍스트가 pane 에 **몇 번에 나눠 제출되는가**(정규화된 제출 단위 목록).
///
/// 원시 바이트 주입(`WriteReq::Program` — 종전 `Data`·바이트 동일)에서 본문 개행은 그대로 Enter 다. 그래서 TUI 는 텍스트를
/// 행 단위로 쪼개 각각 제출하고, 훅은 **행 하나하나**를 프롬프트로 본다. 원장이 전문만 알고
/// 있으면 그 행들은 어느 레코드와도 일치하지 않는다(관통 경로 · 모듈 머리말 R6).
///
/// 규칙: `\n`·`\r` 로 분해(`\r\n` 은 가운데가 빈 조각이 되어 자동 탈락) → 각 행을 `normalize` →
/// 빈 행 제외 → **같은 문장 중복 제거**(원장은 sha 집합으로 소비되므로 중복은 순수 낭비다).
/// 순서는 **첫 출현 순**으로 보존한다 — 감사에서 `part` 인덱스가 원문의 어디쯤인지 가리킨다
/// (중복을 접었으므로 원문 행 번호와 1:1 은 아니다).
pub fn submit_units(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in text.split(['\n', '\r']) {
        let n = normalize(raw);
        if n.is_empty() {
            continue;
        }
        if seen.insert(n.clone()) {
            out.push(n);
        }
    }
    out
}

/// 크기 상한 초과 시 1세대 회전. 실패는 무시(회전 실패가 기록을 막으면 판별이 열린다 —
/// 원장 부재는 곧 게이트 개방 방향이므로, 회전보다 기록 지속이 우선이다).
fn rotate_if_needed(p: &Path) {
    if let Ok(m) = std::fs::metadata(p) {
        if m.len() > LEDGER_MAX_BYTES {
            let _ = std::fs::rename(p, p.with_extension("jsonl.1"));
        }
    }
}

/// ★주입 **직전** 호출 — 배달 사실을 원장에 append 한다.
///
/// 호출 규약(불변식 ①): 반드시 `write_tx.try_send(..)` **직전**에 부른다. try_send 가 실패해
/// 실제 주입이 없었더라도 원장에 남는 것은 무해하다 — 그 방향의 오류는 '오너 문장이 기계로
/// 오인될 수 있음'(경미)이고, 반대(주입은 됐는데 원장에 없음)는 게이트 개방(치명)이다.
///
/// ★호출부는 되도록 `record_audited` 를 쓴다 — 실패를 이벤트로 남겨야 흔적이 생긴다.
///
/// ★R6 이후 **테스트 전용**이다(선례: `digest`). 생산 경로는 조각 기록의 성패까지 이벤트로
/// 드러내야 하므로 `record_audited` → `record_full` 을 쓴다. 여기 남겨 둔 이유는 "전문 레코드
/// 하나만 보면 되는" 기존 회귀 테스트의 가독성이며, 삭제하면 그 테스트들이 보고 싶지 않은
/// 조각 필드까지 열게 된다. (`#[cfg(test)]` 을 붙이는 이유는 신규 `dead_code` 경고 0 규약.)
#[cfg(test)]
pub fn record(
    socket_path: &Path,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
) -> Outcome {
    record_full(socket_path, surface_id, text, origin, from_surface).outcome
}

/// `record_full` 의 결과 — 전문 레코드의 성패에 **조각(R6) 기록의 성패**를 더한 것.
///
/// 조각이 빠지면 그 행이 제출될 때 층1 이 미스한다(= 게이트 개방 방향). 차단할 수는 없으니
/// (주입을 막으면 배달이 죽는다) 호출부가 이벤트로 드러낼 수 있게 사실을 그대로 돌려준다.
#[derive(Debug)]
pub struct RecordReport {
    /// 전문 레코드의 결과(종전 `record` 의 반환값과 동일 의미).
    pub outcome: Outcome,
    /// 실제로 남긴 제출 단위 조각 수(전문 레코드 제외).
    pub parts_written: usize,
    /// 상한(`MAX_PARTS`)에 걸려 **남기지 못한** 조각 수. 0 이 아니면 그 행들은 층1 미대조다.
    pub parts_dropped: usize,
    /// 조각 append 실패 사유(있으면 그 이후 조각이 없다).
    pub parts_failed: Option<String>,
}

/// ★주입 **직전** 호출(감사 상세 포함) — 전문 + 제출 단위 조각을 원장에 남긴다.
///
/// 호출 규약·불변식은 `record` 와 같다. 조각의 의미와 대안 비교는 모듈 머리말 R6 절.
#[cfg(test)]
pub fn record_full(
    socket_path: &Path,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
) -> RecordReport {
    record_full_with(socket_path, surface_id, text, origin, from_surface, &Value::Null)
}

/// `record_full` + 추가 사실 병합(계약은 `record_audited_with` doc 참조).
pub fn record_full_with(
    socket_path: &Path,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
    extra: &Value,
) -> RecordReport {
    let blank = |o: Outcome| RecordReport {
        outcome: o,
        parts_written: 0,
        parts_dropped: 0,
        parts_failed: None,
    };
    let norm = normalize(text);
    if norm.is_empty() {
        // 공백뿐 — 프롬프트가 될 수 없다(훅도 빈 프롬프트를 판정하지 않는다)
        return blank(Outcome::Blank);
    }
    let p = ledger_path(socket_path);
    if let Some(d) = p.parent() {
        if let Err(e) = std::fs::create_dir_all(d) {
            return blank(Outcome::Failed(format!("상태 디렉터리 생성 실패: {e}")));
        }
    }
    rotate_if_needed(&p);
    let epoch = crate::state::now_epoch();
    let preview: String = norm.chars().take(PREVIEW_CHARS).collect();
    let sha = digest_normalized(&norm);
    // ★R6: 실제 제출 단위. 단일 행이면 units==1 이고 조각은 0 건이다(종전과 동일한 원장 모양).
    let units = submit_units(text);
    let parts: Vec<&String> = units.iter().filter(|u| **u != norm).collect();
    let dropped = parts.len().saturating_sub(MAX_PARTS);
    let mut rec = json!({
        "v": LEDGER_SCHEMA,
        // ★surface 는 pane env `CYS_SURFACE_ID` 와 **같은 표기**(정수 문자열)다 —
        //   판독자가 재조립 없이 문자열 비교로 조인한다(state.rs: builder.env(ENV_SURFACE_ID, id)).
        "surface": surface_id.to_string(),
        "ts_epoch": epoch,
        "ts": iso_utc(epoch),
        "sha256": sha,
        "origin": origin.as_str(),
        "from": from_surface.map(|s| s.to_string()),
        "chars": norm.chars().count(),
        "preview": preview,
        // ★R6: 이 배달이 몇 번에 나뉘어 제출되는가. 1 이면 쪼개질 수 없다 —
        //   판독자가 "프롬프트가 이 레코드의 조각인가" 를 물을 필요조차 없는 레코드다.
        "units": units.len(),
    });
    // ★B1: 호출부가 아는 사실을 병합 — 기존 키는 절대 덮지 않는다(스키마 계약).
    if let (Some(add), Some(dst)) = (extra.as_object(), rec.as_object_mut()) {
        for (k, v) in add {
            dst.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    if dropped > 0 {
        // ★R7 — **판독자가 볼 수 있는 자리**에 초과 사실을 남긴다.
        //   종전에는 데몬 버스 이벤트(`delivery.parts_incomplete`)뿐이었고, 임무 게이트는 버스를
        //   구독하지 않으므로 초과가 나도 임무 verdict 에 흔적이 0 이었다 — 즉 "원장에 없는 행이
        //   오너 임무를 발급"하는 경로가 **조용히** 열려 있었다(SOT §4-7 ⓑ).
        //   필드가 붙으면 판독자는 그 배달에 대한 판정을 접고(fail-closed) 이상징후로 고지한다.
        //   ★구 판독자 호환: 모르는 필드는 무시되므로 스큐에서도 종전 동작 그대로다(회귀 0).
        rec["parts_capped"] = json!(dropped);
    }
    // flush 까지 마친 뒤에만 호출자가 try_send 로 넘어간다(불변식 ①).
    let outcome = append_line(&p, &rec);
    if !matches!(outcome, Outcome::Recorded) || parts.is_empty() {
        let mut r = blank(outcome);
        r.parts_dropped = dropped;
        return r;
    }
    let recs: Vec<Value> = parts
        .iter()
        .take(MAX_PARTS)
        .enumerate()
        .map(|(i, u)| {
            // ★R7 경량화 — 조각 레코드는 **판독자 필수 필드 + 감사 결속**만 남긴다.
            //   전문 레코드에 있는 `ts`(ISO 문자열)·`from` 은 조각에서 뺐다: 판독자
            //   (`javis_mission.read_delivery`)가 읽는 필드는 v·surface·ts_epoch·sha256·chars·
            //   preview·origin·units·part·parent 뿐이고, `ts` 는 `ts_epoch` 에서 유도되며 `from`
            //   은 같은 배달의 전문 레코드에 그대로 있다(같은 `ts_epoch`·`parent` 로 결속된다).
            //   빼는 이유는 미학이 아니라 **회전 예산**이다 — 조각 1건당 약 42 B, 디렉티브급
            //   push 1회당 약 30 KB 다(SOT §4-6).
            json!({
                "v": LEDGER_SCHEMA,
                "surface": surface_id.to_string(),
                "ts_epoch": epoch,
                "sha256": digest_normalized(u),
                "origin": origin.as_str(),
                "chars": u.chars().count(),
                "preview": u.chars().take(PART_PREVIEW_CHARS).collect::<String>(),
                // 감사용 결속 — 이 조각이 어느 배달의 몇 번째 제출 단위인가.
                "part": i + 1,
                "parent": sha,
            })
        })
        .collect();
    let n = recs.len();
    let (written, failed) = match append_lines(&p, &recs) {
        Outcome::Recorded => (n, None),
        Outcome::Blank => (n, None), // 도달 불가(빈 슬라이스 아님) — 방어적
        Outcome::Failed(why) => (0, Some(why)),
    };
    RecordReport {
        outcome,
        parts_written: written,
        parts_dropped: dropped,
        parts_failed: failed,
    }
}

/// ★감사 포함 기록 — 실패를 **이벤트로 남긴다**(OUT OF SCOPE 대응: 막을 수 없는 것을 보이게).
///
/// 기록 실패는 조용히 지나가면 안 된다. 그 순간 주입된 텍스트는 원장에 없으므로 훅에게
/// **오너 임무로 보이고**, 게이트가 열린다. 차단할 수는 없다(주입을 막으면 배달 자체가 죽는다)
/// — 대신 `delivery.record_failed` 이벤트 + 데몬 로그로 흔적을 남겨 사후에 반드시 드러나게 한다.
/// 반환값은 종전 호출부 호환을 위한 bool(기록됨=true).
pub fn record_audited(
    daemon: &crate::state::Daemon,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
) -> bool {
    record_audited_with(daemon, surface_id, text, origin, from_surface, &Value::Null)
}

/// ★B1(0.14.30): `record_audited` + **호출부가 아는 추가 사실**을 같은 레코드에 실는다.
///
/// 왜 필요한가(queue-starvation-case.md §4-ⓓ): 원장에는 배달 시각만 있고 **enqueue 시각이
/// 없어** 사후에 전수 지연을 계산할 수 없었다(그 문서의 표본이 18건에 그친 이유). 큐 배달만
/// 아는 사실(항목 id·enqueue 시각·대기초)을 여기서 실어 원장 한 파일로 전수 측정이 되게 한다.
///
/// `extra` 는 객체여야 하며 **기존 키를 덮지 않는다**(충돌 키는 무시 — 스키마 계약 보호).
/// 객체가 아니면(Null 등) 종전 레코드와 바이트 동일하다.
pub fn record_audited_with(
    daemon: &crate::state::Daemon,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
    extra: &Value,
) -> bool {
    let report =
        record_full_with(&daemon.socket_path, surface_id, text, origin, from_surface, extra);
    // ★R6: 조각(제출 단위) 기록이 불완전하면 그 행들은 층1 미대조다 — 차단할 수 없으니 드러낸다.
    if report.parts_dropped > 0 || report.parts_failed.is_some() {
        let path = ledger_path(&daemon.socket_path);
        daemon.bus.publish(
            "delivery.parts_incomplete",
            "system",
            Some(surface_id),
            json!({
                "origin": origin.as_str(),
                "written": report.parts_written,
                "dropped": report.parts_dropped,
                "cap": MAX_PARTS,
                "reason": report.parts_failed,
                "path": path.to_string_lossy(),
                "impact": "이 배달의 일부 행이 배달 원장에 없다 — 그 행이 단독 제출되면 임무 \
                           게이트가 기계 push 를 오너 임무로 오인할 수 있다(오너 보고 대상). \
                           ★R7: 상한 초과(dropped>0)는 전문 레코드의 `parts_capped` 필드로도 \
                           남아 임무 게이트가 그 배달에 대한 판정을 접는다(fail-closed) — 이 \
                           이벤트는 데몬 버스 구독자용이고, 게이트가 보는 것은 원장 쪽이다."
            }),
        );
    }
    match report.outcome {
        Outcome::Recorded => true,
        Outcome::Blank => false,
        Outcome::Failed(why) => {
            let path = ledger_path(&daemon.socket_path);
            eprintln!(
                "cysd: ★배달 원장 기록 실패 — surface={surface_id} origin={} path={} 사유={why} \
                 (이 주입은 원장에 없어 임무 게이트가 오너 입력으로 오인할 수 있다)",
                origin.as_str(),
                path.display()
            );
            daemon.bus.publish(
                "delivery.record_failed",
                "system",
                Some(surface_id),
                json!({
                    "origin": origin.as_str(),
                    "reason": why,
                    "path": path.to_string_lossy(),
                    "impact": "이 주입은 배달 원장에 없다 — 임무 게이트가 기계 push 를 오너 임무로 \
                               오인할 수 있다(자율 착수 권한 오발급 위험). 오너에게 보고 대상."
                }),
            );
            false
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // ══════════════════════════════════════════════════════════════════════════
    // ★R5-B 테스트 상태 격리 — "라이브 원장을 테스트가 더럽히지 않는다"
    //
    // ## 결함(실측)
    // `pack_state_dir()` 는 `CYS_STATE_DIR` 이 없으면 실 HOME 으로 해소된다. 그래서 원장을
    // 건드리는 테스트가 격리를 **잊으면** `~/.cys/state/delivery-<임시소켓>.jsonl` 이 진짜로
    // 생긴다(1회 실행당 9개 · 누적 67개 실측). 원장 배선이 `record_audited` 로 넓어지며
    // 악화됐고, 앞으로 더 넓어질수록 "잊을 자리"도 함께 늘어난다.
    //
    // ## 그래서 방어를 개별 테스트가 아니라 **해소 함수 자체**에 둔다 (해소 우선순위)
    //   ① 이 테스트 스레드의 격리 루트(`isolate_state_dir*`)
    //   ② `CYS_STATE_DIR`(러너 전역 지정)
    //   ③ 그것도 없으면 — **실 HOME 이 아니라** 프로세스별 temp 샌드박스
    // ③ 이 본체다. 개별 테스트가 잊어도 라이브는 절대 안 더러워진다.
    //
    // ## 왜 env 가 아니라 thread-local 인가 (①)
    // `CYS_STATE_DIR` 은 **프로세스 전역**이라 병렬 러너에서 서로를 덮어쓴다 — 그래서 종전
    // `with_state_dir` 은 전역 뮤텍스로 배터리 전체를 직렬화해야 했다. 공용 하네스
    // (`channels::tests::tmp_daemon` 51곳 · `handlers::tests::daemon_with_acl` 15곳)에 그 방식을
    // 그대로 확대하면 ⓐ 66개 테스트가 직렬화되고 ⓑ 기존 `ACL_ENV_LOCK` 과의 **획득 순서**가
    // 갈려 교착이 열린다(실제로 기존 두 테스트는 STATE→ACL 순, 하네스 경유는 ACL→STATE 순이
    // 된다). thread-local 은 락이 없어 두 문제를 **원천적으로 만들지 않는다**.
    //
    // ## ★관측성 — 우선순위가 감사자를 속이지 않게 (R6 · 2026-08-02)
    // ① 이 thread-local 은 `CYS_STATE_DIR` **보다 우선**이다. 그래서 감사자가
    // `CYS_STATE_DIR=<스크래치> cargo test` 로 "테스트가 원장을 어디에 쓰는지 보자"고 해도,
    // 격리된 테스트(대부분)는 그 스크래치가 아니라 `$TMPDIR` 샌드박스에 썼다 — **지정한
    // 디렉터리가 텅 비어 보이고**, 그것은 "테스트가 원장을 안 건드렸다"로 오독된다.
    // 우선순위를 바꾸면(env 가 이기게 하면) ① 의 병렬성·무락 성질이 죽으므로, 대신
    // **샌드박스 루트 자체가 러너 지정 디렉터리를 존중**하게 했다(`sandbox_root_for`):
    //   · `CYS_STATE_DIR` 지정 → 격리분은 `<지정>/iso-*/`, 격리 잊은 분은 `<지정>/` 최상위
    //   · 미지정               → 종전대로 `$TMPDIR/cys-test-state-<pid>/`
    // 즉 **우선순위는 그대로 두고 목적지를 합쳤다** — 감사자는 자기가 지정한 한 곳만 보면 된다.
    //
    // ## 정직 고지 — 이 층이 못 하는 것
    //  · 데몬이 **배경 스레드**에서 쓰는 원장은 thread-local 을 보지 못한다. 그 경로는 ②/③ 으로
    //    내려가며, 라이브 오염을 막는 것은 ③ 이다(격리 정밀도는 떨어지고 안전성은 유지된다).
    //  · 하드 실패(panic)는 **채택하지 않았다**. 배경 스레드에서의 panic 은 조용히 삼켜지거나
    //    (`join().ok()`) 뮤텍스를 poison 시켜 무관한 테스트를 무너뜨린다 — 즉 탐지기로서
    //    신뢰할 수 없고 새 flakiness 원인이 된다. 대신 **탐지를 결정론으로** 돌린다:
    //    격리를 잊은 쓰기는 전부 샌드박스 루트 **최상위**에 떨어지므로, 아래 한 줄이 회귀 게이트다.
    //      `ls "$CYS_STATE_DIR"/delivery-*.jsonl | wc -l`            → **0 이어야 한다**
    //      (미지정이면 `ls "$TMPDIR"/cys-test-state-*/delivery-*.jsonl | wc -l`)
    //    (배경 스레드 위반까지 잡는다 — panic 방식은 못 잡는 범주다.)
    //  · `CYS_STATE_DIR` 을 **라이브**(`~/.cys/state`)로 지정하면 테스트가 라이브를 쓴다. 그건
    //    러너의 명시 선택이며 종전(②)도 같았다 — 대신 아래
    //    `test_build_never_resolves_state_root_to_live_home` 이 **하드 실패**로 고함친다.
    // ══════════════════════════════════════════════════════════════════════════

    thread_local! {
        /// 이 테스트 스레드 전용 원장 루트(①). `None` 이면 ②→③ 으로 내려간다.
        static THREAD_STATE_ROOT: std::cell::RefCell<Option<PathBuf>> =
            const { std::cell::RefCell::new(None) };
    }

    /// `pack_state_dir()` 가 최우선으로 참조하는 훅(①).
    pub(crate) fn thread_state_root() -> Option<PathBuf> {
        THREAD_STATE_ROOT.with(|c| c.borrow().clone())
    }

    /// 샌드박스 루트 해소의 **순수 함수**(env 를 읽지 않는다 — 그래야 병렬 러너를 건드리지 않고
    /// 회귀 핀을 걸 수 있다). `runner_dir` = `CYS_STATE_DIR` 원문.
    ///
    /// ★R6 관측성: 러너가 디렉터리를 지정했으면 **그곳을 쓴다**. thread-local 격리(①)가
    /// `CYS_STATE_DIR`(②)보다 우선이라, 종전엔 감사자가 지정한 스크래치가 텅 빈 채로 남아
    /// "테스트가 원장을 안 건드렸다"로 오독됐다. 우선순위는 그대로 두고 **목적지를 합친다.**
    pub(crate) fn sandbox_root_for(runner_dir: Option<&str>) -> PathBuf {
        match runner_dir {
            Some(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
            _ => std::env::temp_dir().join(format!("cys-test-state-{}", std::process::id())),
        }
    }

    /// 이 테스트 프로세스의 샌드박스 루트 = **감사 루트**. 격리 디렉터리(①)의 부모이자,
    /// 격리를 **잊은** 쓰기(③)가 떨어지는 자리다. 최상위에 `delivery-*.jsonl` 이 있으면
    /// = 잊은 테스트가 있다(격리분은 `iso-*/` 하위로 들어가므로 최상위에 안 나온다).
    pub(crate) fn test_sandbox_root() -> PathBuf {
        let p = sandbox_root_for(std::env::var("CYS_STATE_DIR").ok().as_deref());
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// ③ 최종 방어선 — 실 HOME 대신 이 경로를 돌려준다. 프로세스당 1회 stderr 로 고지한다
    /// (`cargo test -- --nocapture` 에서 보인다).
    ///
    /// ★이 고지 자체는 **위반 신호가 아니다.** 아래 R5-B 자기검증 테스트 둘이 이 경로를
    /// 일부러 해소하므로 정상 실행에서도 반드시 1회 뜬다. 판정은 **경로 해소**가 아니라
    /// **실제 쓰기**로 한다 — 권위 있는 회귀 게이트는 주석 머리말의 파일 개수 한 줄이다.
    pub(crate) fn unisolated_sandbox_root() -> PathBuf {
        let p = test_sandbox_root();
        static WARN: std::sync::Once = std::sync::Once::new();
        WARN.call_once(|| {
            eprintln!(
                "cysd(test): CYS_STATE_DIR 미설정 — 배달 원장을 실 HOME(~/.cys/state) 대신 \
                 {} 로 강제 격리한다(R5-B). ★위반 판정은 이 줄이 아니라 다음 개수로 한다: \
                 이 디렉터리 **최상위**의 delivery-* 가 0 이 아니면 격리를 잊은 테스트가 있다 \
                 (delivery::tests::isolate_state_dir* 를 그 테스트/하네스에 적용할 것).",
                p.display()
            );
        });
        p
    }

    /// 격리 디렉터리 이름 충돌 방지용 일련번호(스레드 재사용·동일 tag 중복 호출 대비).
    static ISO_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn new_isolated_dir(tag: &str) -> PathBuf {
        let n = ISO_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let slug: String = tag
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let d = test_sandbox_root().join(format!("iso-{n:04}-{slug}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("격리 상태 디렉터리 생성");
        d
    }

    /// 이 테스트 스레드의 원장 루트를 새 임시 디렉터리로 고정한다(①). 가드가 drop 되면
    /// 이전 값으로 복원하고 디렉터리를 지운다 — **가드를 붙들 자리가 있는 테스트**용.
    pub(crate) fn isolate_state_dir(tag: &str) -> StateDirGuard {
        let dir = new_isolated_dir(tag);
        let prev = THREAD_STATE_ROOT.with(|c| c.borrow_mut().replace(dir.clone()));
        StateDirGuard { dir, prev }
    }

    /// 가드를 돌려줄 자리가 없는 **공용 하네스**(`tmp_daemon`·`daemon_with_acl`)용 —
    /// 이 테스트 스레드가 끝날 때까지 유효한 격리 루트를 설정한다(복원 없음 · 호출 시마다 새 dir).
    /// libtest 는 테스트마다 스레드를 새로 띄우므로 스레드 종료가 곧 해제이고, 스레드가 재사용
    /// 되더라도 다음 하네스 호출이 새 디렉터리로 **덮어쓴다**(오염된 값이 살아남지 않는다).
    pub(crate) fn isolate_state_dir_for_thread(tag: &str) -> PathBuf {
        let dir = new_isolated_dir(tag);
        THREAD_STATE_ROOT.with(|c| *c.borrow_mut() = Some(dir.clone()));
        dir
    }

    /// `isolate_state_dir` 의 RAII 가드.
    pub(crate) struct StateDirGuard {
        dir: PathBuf,
        prev: Option<PathBuf>,
    }

    impl StateDirGuard {
        pub(crate) fn path(&self) -> &Path {
            &self.dir
        }
    }

    impl Drop for StateDirGuard {
        fn drop(&mut self) {
            THREAD_STATE_ROOT.with(|c| *c.borrow_mut() = self.prev.clone());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// `record` 는 이제 3상(Recorded/Blank/Failed)을 돌려준다 — "기록됐는가"만 보는 축약.
    fn rec_ok(o: Outcome) -> bool {
        matches!(o, Outcome::Recorded)
    }

    fn with_state_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        let g = isolate_state_dir("delivery-unit");
        let dir = g.path().to_path_buf();
        f(&dir)
    }

    /// ★R5-B 회귀: **테스트 빌드는 실 HOME 으로 절대 해소되지 않는다.**
    /// 이것이 깨지면 격리를 잊은 테스트 하나가 곧바로 라이브 원장(`~/.cys/state/delivery-*.jsonl`)
    /// 을 만든다 — 실제로 67개가 그렇게 쌓였다(제품 결함이 아니라 테스트 위생 결함이지만,
    /// 오너의 라이브 상태를 오염시키므로 방어는 코드에 있어야 한다).
    ///
    /// ★R6: 러너가 `CYS_STATE_DIR` 을 **라이브로** 겨누면 여기서 하드 실패한다 — 종전에는
    /// thread-local 우선순위 덕에 조용히 지나갔지만, 그건 "안전"이 아니라 **오조준의 은폐**였다.
    #[test]
    fn test_build_never_resolves_state_root_to_live_home() {
        let live_root = cys::home_dir().join(".cys");
        let got = default_state_root();
        assert!(
            !got.starts_with(&live_root),
            "테스트 빌드의 기본 상태 루트가 라이브 HOME 안이다: {} (라이브: {}) — \
             CYS_STATE_DIR 이 라이브를 겨누고 있지 않은지 확인하라",
            got.display(),
            live_root.display()
        );
        assert_eq!(got, test_sandbox_root(), "기본 루트는 감사 루트와 같아야 한다");
        // 러너 미지정일 때만 temp 아래다(지정 시엔 지정한 곳이 감사 루트 — 위 라이브 가드가 지킨다).
        assert!(
            sandbox_root_for(None).starts_with(std::env::temp_dir()),
            "러너 미지정 샌드박스는 temp 아래여야 한다"
        );
    }

    /// ★R6 회귀 핀 — **감사 가능성**: 러너가 지정한 디렉터리가 곧 감사 루트여야 한다.
    ///
    /// 결함(관측성): thread-local 격리(①)가 `CYS_STATE_DIR`(②)보다 우선이라, 감사자가
    /// `CYS_STATE_DIR=<스크래치> cargo test` 로 돌려도 격리된 테스트는 `$TMPDIR` 에 썼다 →
    /// 지정한 스크래치가 **텅 비어** "테스트가 원장을 안 건드렸다"로 오독된다. 우선순위는
    /// 유지하고 목적지만 합쳤으므로, 그 합류가 깨지면 여기서 잡힌다.
    ///
    /// ★env 를 **쓰지 않고 읽기만** 한다(set_var 는 프로세스 전역이라 병렬 러너를 오염시킨다).
    /// 그 대가로 핵심 단언(감사 루트 == 러너 지정 디렉터리)은 `CYS_STATE_DIR` 이 **설정된
    /// 실행에서만** 발동한다 — 정직 고지다. 이 저장소의 규약 실행은 항상 지정이며
    /// (`CYS_STATE_DIR=<스크래치> cargo test`), 지정이 없으면 애초에 "감사자가 지정한 곳"이
    /// 존재하지 않아 검사할 대상이 없다.
    #[test]
    fn isolation_dirs_live_under_runner_specified_audit_root() {
        assert_eq!(
            sandbox_root_for(Some("/tmp/cys-audit-scratch")),
            PathBuf::from("/tmp/cys-audit-scratch"),
            "러너 지정 디렉터리를 존중해야 한다"
        );
        // ★배선 핀: 순수 함수가 옳아도 `test_sandbox_root()` 가 그것을 **안 부르면** 의미가 없다.
        //   (이 단언이 없으면 해소를 `sandbox_root_for(None)` 으로 되돌려도 테스트가 통과한다 —
        //    실제로 반사실 대조에서 그렇게 통과하는 것을 확인하고 추가했다.)
        if let Ok(v) = std::env::var("CYS_STATE_DIR") {
            if !v.trim().is_empty() {
                assert_eq!(
                    test_sandbox_root(),
                    PathBuf::from(v.trim()),
                    "러너가 CYS_STATE_DIR 을 지정했는데 감사 루트가 그곳이 아니다 — \
                     감사자가 지정한 스크래치가 비어 보여 '테스트가 원장을 안 건드렸다'로 오독된다"
                );
            }
        }
        assert_eq!(
            sandbox_root_for(Some("   ")),
            sandbox_root_for(None),
            "공백뿐인 지정은 미지정과 같게 다뤄야 한다(pack_state_dir 의 trim 규약과 동형)"
        );
        let audit = test_sandbox_root();
        let g = isolate_state_dir("audit-visibility");
        assert!(
            g.path().starts_with(&audit),
            "격리 디렉터리가 감사 루트 밖이다 — 감사자가 지정한 곳이 비어 보인다: {} (감사 루트: {})",
            g.path().display(),
            audit.display()
        );
        assert!(
            ledger_path(Path::new("/x/cys.sock")).starts_with(&audit),
            "격리 중 원장 경로가 감사 루트 밖이다"
        );
        // 격리분은 하위 디렉터리에 들어간다 = 감사 루트 **최상위**의 delivery-* 개수가
        // '격리를 잊은 쓰기'의 척도라는 계약이 유지된다(모듈 머리말의 회귀 게이트 한 줄).
        assert_ne!(g.path(), audit, "격리 디렉터리가 감사 루트 자신이면 최상위 개수 척도가 죽는다");
    }

    /// ★R5-B 회귀: 스레드 로컬 격리가 실제로 원장 경로를 접고, drop 후 복원되는가.
    /// 소켓→레인 판정 **공유 코퍼스** — python `javis_lane` 이 소비하는 **같은 파일**을
    /// 컴파일 타임에 싣는다. 경로가 바뀌면 여기서 빌드가 깨진다(사본 분화를 컴파일러가 막는다).
    const LANE_CORPUS: &str = include_str!("../../../cysjavis-pack/bin/tests/fixtures/lane-key-corpus.json");

    /// ★H-LANE-2(P0 파리티): 소켓→base 판정이 **공유 코퍼스**와 일치한다.
    ///
    /// H-LANE-ISO 가 사고 재현·회귀 방지를 맡는다면 이 검체는 **2언어 대칭 이탈 탐지기**다 —
    /// python `javis_lane.socket_is_base` 가 같은 파일을 소비하므로, 한쪽만 고쳐지면 그 순간
    /// 한쪽이 적색이 된다. 레인 판정이 갈리면 두 구현이 **서로 다른 원장 파일**을 읽어
    /// 층1 이 조용히 실패한다(오염의 반대 방향 사고).
    #[test]
    fn socket_base_verdict_matches_the_shared_lane_corpus() {
        let c: serde_json::Value =
            serde_json::from_str(LANE_CORPUS).expect("lane-key-corpus.json 판독 불가");
        let cases = c["cases"].as_array().expect("cases 배열 부재");
        assert!(cases.len() >= 17, "레인 코퍼스가 줄었다: {}", cases.len());
        let mut fails: Vec<String> = Vec::new();
        for it in cases {
            let sock = it["sock"].as_str().expect("sock");
            let want = it["is_base"].as_bool().expect("is_base");
            let got = socket_is_base(sock);
            if got != want {
                fails.push(format!(
                    "{sock:?}: 기대 is_base={want} / 실측 {got} — why: {}",
                    it["why"].as_str().unwrap_or("")
                ));
            }
            // lane_key 도 함께 대조한다 — base 면 정확히 "base", 아니면 절대 "base" 가 아니다.
            let k = lane_key(Path::new(sock));
            if want && k != "base" {
                fails.push(format!("{sock:?}: base 인데 lane_key={k}"));
            }
            if !want && k == "base" {
                fails.push(format!("{sock:?}: 비-base 인데 lane_key=base — 본 레인을 공유한다"));
            }
        }
        assert!(fails.is_empty(), "레인 판정 파리티 이탈:\n  - {}", fails.join("\n  - "));
    }

    /// ★H-LANE-ISO(P0 · master 등재 2026-09-04): **격리 소켓이 본 레인을 오염시키지 않는다.**
    ///
    /// 실사고: `~/.cys/state-harness/cys.sock` · `/var/folders/…/l1-new-*/cys.sock` 로 띄운
    /// 데몬이 본 레인 `delivery-base.jsonl`·`.epoch.json` 에 썼다(외부 좌석 레코드 70건 ·
    /// epoch 덮어씀) → 본부 임무 게이트 오탐 폐쇄 + 전 노드 층1 원장 오염.
    ///
    /// 원인: 종전 `socket_is_base` 가 **basename 만** 봐서 디렉터리를 무시했다. 역설적으로
    /// `/tmp/whatever.sock` 처럼 **파일명이 다르면** 올바로 격리됐다 — 즉 **가장 흔한 격리
    /// 방식**(관례 파일명 유지 + 디렉터리 분리)만 조용히 실패했다.
    ///
    /// ★이 검체는 **음성 대조군을 내장**한다: 구 규칙(basename 판정)을 그 자리에서 재현해
    /// 오염이 **실제로 일어났음**을 먼저 보이고, 신 규칙이 그것을 막는 것을 보인다. 그래야
    /// "이 검체가 무엇을 지키는지"가 검체 안에서 자명하다.
    #[test]
    fn isolated_socket_never_folds_into_the_base_lane() {
        // 구 규칙 재현 — basename 만 보던 판정(음성 대조군).
        let old_rule = |sock: &str| -> bool {
            let norm = sock.replace('\\', "/");
            if norm.split('/').any(|p| p.starts_with("cys-dept-")) {
                return false;
            }
            let last = norm.rsplit('/').next().unwrap_or("");
            last == "cys" || last == "cys.sock"
        };

        // 실사고에서 관측된 격리 소켓 2종.
        let leaky = [
            "/Users/x/.cys/state-harness/cys.sock",
            "/var/folders/ab/T/l1-new-1234/cys.sock",
        ];
        for sock in leaky {
            // ① 음성 대조 — 구 규칙에서는 **base 로 접혔다**(사고 재현).
            assert!(
                old_rule(sock),
                "음성 대조군이 성립하지 않는다 — 구 규칙에서 이 소켓이 base 가 아니면 \
                 이 검체는 사고를 재현하지 못한다: {sock}"
            );
            // ② 신 규칙 — 자기 레인이다.
            assert!(!socket_is_base(sock), "격리 소켓이 아직도 base 로 접힌다: {sock}");
            let k = lane_key(Path::new(sock));
            assert_ne!(k, "base", "레인 키가 base 다 — 본 레인 파일을 공유한다: {sock}");
            // ③ 그래서 원장·epoch 파일이 **본 레인과 다른 이름**이다.
            let led = ledger_path(Path::new(sock));
            let ep = epoch_path(Path::new(sock));
            let base_led = ledger_path(Path::new("/Users/x/.local/state/cys/cys.sock"));
            let base_ep = epoch_path(Path::new("/Users/x/.local/state/cys/cys.sock"));
            assert_ne!(led, base_led, "격리 데몬이 본 레인 원장에 쓴다: {}", led.display());
            assert_ne!(ep, base_ep, "격리 데몬이 본 레인 epoch 을 덮는다: {}", ep.display());
            assert!(
                led.file_name().unwrap().to_string_lossy().starts_with("delivery-"),
                "원장 파일명 규약이 깨졌다: {}", led.display()
            );
        }

        // ④ 하위호환 — **진짜 기본 소켓은 여전히 base** 다(기존 delivery-base.jsonl 유효).
        for ok in ["/Users/x/.local/state/cys/cys.sock", "/home/u/.local/state/cys/cys.sock", ""] {
            assert!(socket_is_base(ok), "기본 소켓이 base 에서 빠졌다 — 기존 원장이 고아가 된다: {ok:?}");
        }
        // ⑤ 종전에 이미 비-base 였던 것들은 그대로다(회귀 없음).
        for no in ["/tmp/whatever.sock", "/Users/x/.local/state/cys-dept-sales/cys.sock"] {
            assert!(!socket_is_base(no), "비-base 판정이 뒤집혔다: {no}");
        }
        // ⑥ ★부모 이름은 **정확히** `cys` 여야 한다 — 접두 비교로 완화하면 `cys` 로 시작하는
        //    아무 디렉터리나 base 특권을 얻는다(mutation P-M2 가 이 사각을 드러냈다).
        //    `cys-dept-` 는 앞선 가드가 먼저 잡으므로 그것만으로는 이 자리를 못 잰다.
        for near in [
            "/Users/x/.local/state/cys-harness/cys.sock",
            "/Users/x/.local/state/cystest/cys.sock",
            "/Users/x/.local/state/cys2/cys.sock",
            "/Users/x/.cys/cys.sock",
        ] {
            assert!(
                !socket_is_base(near),
                "부모 이름이 `cys` 가 아닌데 base 로 접혔다 — 접두 비교로 완화된 것 같다: {near}"
            );
        }
        // ⑥ 기계 독립 — 이 판정은 **경로 모양**만 본다(실제 홈·존재 여부 무관).
        //    그래야 python 과 공유하는 소켓→lane_key 매트릭스가 기계마다 같은 답을 낸다.
        assert!(socket_is_base("/nonexistent/machine/.local/state/cys/cys.sock"));
    }

    /// (env 를 건드리지 않으므로 병렬 러너에서 다른 테스트와 간섭하지 않는다 — 그것이 채택 이유다.)
    #[test]
    fn isolate_state_dir_scopes_paths_to_this_thread_and_restores() {
        let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
        let before = pack_state_dir();
        {
            let g = isolate_state_dir("scope-check");
            assert_eq!(pack_state_dir(), g.path(), "격리 중에는 가드 경로가 이긴다");
            assert!(
                ledger_path(sock).starts_with(g.path()),
                "원장 경로가 격리 디렉터리 밖이다: {}",
                ledger_path(sock).display()
            );
            assert!(
                epoch_path(sock).starts_with(g.path()),
                "epoch 표식도 같은 격리 안에 있어야 한다"
            );
            // 중첩 격리도 성립해야 한다(하네스가 이미 격리한 위에 테스트가 다시 격리하는 형태).
            let inner_dir = {
                let g2 = isolate_state_dir("scope-check-inner");
                assert_eq!(pack_state_dir(), g2.path());
                g2.path().to_path_buf()
            };
            assert_ne!(inner_dir, g.path());
            assert_eq!(pack_state_dir(), g.path(), "중첩 가드 drop 후 바깥 격리로 복원");
        }
        assert_eq!(pack_state_dir(), before, "가드 drop 후 원래 루트로 복원돼야 한다");
    }

    #[test]
    fn normalize_collapses_all_unicode_whitespace() {
        assert_eq!(normalize("  a \t\n b  "), "a b");
        assert_eq!(normalize("[wakeup]\n다음 액션 착수"), "[wakeup] 다음 액션 착수");
        // NBSP·ZWSP 아닌 유니코드 공백류도 접힌다(ideographic space U+3000)
        assert_eq!(normalize("a\u{3000}b"), "a b");
        assert_eq!(normalize("\u{00a0}x\u{00a0}"), "x");
        assert_eq!(normalize("   "), "");
        // 대소문자·유니코드 합성은 건드리지 않는다(과확장 금지)
        assert_eq!(normalize("Abc"), "Abc");
    }

    #[test]
    fn digest_is_whitespace_insensitive_but_content_sensitive() {
        assert_eq!(digest("다음 액션 착수"), digest("  다음   액션\t착수 \n"));
        assert_ne!(digest("다음 액션 착수"), digest("다음 액션 착수2"));
        // 알려진 벡터 — 판독자(python)와의 교차검증 앵커
        assert_eq!(
            digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha1_matches_standard_vectors() {
        // FIPS 180-1 부록 시험벡터 — python hashlib.sha1 과의 동치를 박제한다.
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 64바이트 경계(패딩이 블록을 하나 더 만드는 지점)
        assert_eq!(
            sha1_hex(&[b'a'; 64]),
            "0098ba824b5c16427bd7a1122a5a442a25ec644d"
        );
    }

    #[test]
    fn long_socket_path_key_uses_sha1_tail_like_pack() {
        // python: raw[:120] + "-" + sha1(raw)[:16]  (raw = 새니타이즈 결과 · 앞뒤 '_' strip 뒤)
        // 비-base 여야 이 분기에 온다 → cys-dept- 성분을 넣는다.
        let long = format!("/{}/cys-dept-x/cys.sock", "d".repeat(200));
        let k = lane_key(Path::new(&long));
        assert_eq!(k.chars().count(), 120 + 1 + 16);
        let raw = format!("{}_cys-dept-x_cys.sock", "d".repeat(200));
        assert!(k.ends_with(&sha1_hex(raw.as_bytes())[..16]), "sha1 꼬리 불일치: {k}");
        assert!(k.starts_with(&"d".repeat(120)));
        // ★교차언어 앵커: 아래 리터럴은 `javis_bootstrap.lane_key(동일 입력)` 실측값이다.
        //   양쪽이 갈리면 원장이 **조용히** 무력화되므로 값 자체를 박제한다.
        assert_eq!(k, format!("{}-24618710175be9a6", "d".repeat(120)));
    }

    #[test]
    fn lane_key_mirrors_pack_contract() {
        // ※ 기대값은 `javis_bootstrap.lane_key` 실행 결과와 대조해 박았다(교차검증 스크립트).
        assert_eq!(lane_key(Path::new("/Users/x/.local/state/cys/cys.sock")), "base");
        assert_eq!(
            lane_key(Path::new("/Users/x/.local/state/cys-dept-dept-1/cys.sock")),
            "Users_x_.local_state_cys-dept-dept-1_cys.sock" // python 은 앞뒤 '_' 를 strip 한다
        );
        // 커스텀 소켓(비-base·비-dept)도 레인 유일 키를 받는다
        assert_eq!(lane_key(Path::new("/tmp/whatever.sock")), "tmp_whatever.sock");
        // Windows named pipe — basename 판정 보존
        assert_eq!(lane_key(Path::new(r"\\.\pipe\cys")), "base");
        assert_eq!(
            lane_key(Path::new(r"\\.\pipe\cys-dept-a")),
            "._pipe_cys-dept-a"
        );
    }

    #[test]
    fn ledger_path_is_always_lane_suffixed_under_pack_state_dir() {
        with_state_dir(|td| {
            let p = ledger_path(Path::new("/Users/x/.local/state/cys/cys.sock"));
            assert_eq!(p, td.join("delivery-base.jsonl"), "팩 계약 경로+레인 접미");
            let d = ledger_path(Path::new("/Users/x/.local/state/cys-dept-a/cys.sock"));
            assert_ne!(d, p, "부서 레인은 base 원장과 분리된다");
        });
    }

    #[test]
    fn record_appends_matchable_line_and_skips_blank() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            assert!(rec_ok(record(sock, 7, "[wakeup] 다음 액션 착수", Origin::Send, Some(3))));
            assert!(!rec_ok(record(sock, 7, "   \n ", Origin::Send, None)), "공백뿐이면 미기록");
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            let lines: Vec<&str> = body.lines().collect();
            assert_eq!(lines.len(), 1);
            let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
            assert_eq!(v["surface"], "7");
            assert_eq!(v["origin"], "send");
            assert_eq!(v["from"], "3");
            assert_eq!(v["sha256"], digest("[wakeup]  다음 액션 착수"));
            assert!(v["ts_epoch"].as_f64().unwrap() > 0.0);
        });
    }

    #[test]
    fn record_rotates_at_cap() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let p = ledger_path(sock);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, vec![b'x'; (LEDGER_MAX_BYTES + 1) as usize]).unwrap();
            assert!(rec_ok(record(sock, 1, "hello", Origin::Send, None)));
            assert!(p.with_extension("jsonl.1").exists(), "1세대 회전");
            let body = std::fs::read_to_string(&p).unwrap();
            assert_eq!(body.lines().count(), 1, "회전 후 새 파일에 1건");
        });
    }

    /// ★불변식 ① 실증(race 봉쇄): writer 가 PTY 에 바이트를 쓰는 **그 순간** 원장에 이미
    /// 해당 레코드가 있어야 한다. 호출 규약(record → try_send)이 지켜지면 writer 스레드는
    /// 채널 수신 이후에만 쓰므로 구조적으로 보장된다 — 이 테스트가 그 순서를 박제한다.
    /// 규약을 뒤집으면(try_send 후 record) 아래 `seen_in_ledger` 가 false 로 떨어진다.
    #[test]
    fn ledger_record_strictly_precedes_pty_write() {
        use crate::state::WriteReq;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::sync_channel;
        use std::sync::{Arc, Mutex};

        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "다음 액션 착수"; // ★라벨 없는 사고 문안 그대로
            let want = digest(text);
            let seen_in_ledger = Arc::new(AtomicBool::new(false));
            let wrote = Arc::new(AtomicBool::new(false));

            struct Probe {
                path: PathBuf,
                want: String,
                seen: Arc<AtomicBool>,
                wrote: Arc<AtomicBool>,
            }
            impl std::io::Write for Probe {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    // PTY 쓰기 시점에 원장을 읽는다 — 여기서 이미 보여야 한다.
                    let body = std::fs::read_to_string(&self.path).unwrap_or_default();
                    if body.contains(&self.want) {
                        self.seen.store(true, Ordering::SeqCst);
                    }
                    self.wrote.store(true, Ordering::SeqCst);
                    Ok(buf.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let (tx, rx) = sync_channel::<WriteReq>(1);
            let stop = Arc::new(AtomicBool::new(false));
            let probe = Probe {
                path: ledger_path(sock),
                want: want.clone(),
                seen: Arc::clone(&seen_in_ledger),
                wrote: Arc::clone(&wrote),
            };
            let h = std::thread::spawn(move || crate::state::run_writer_loop(probe, rx, stop));

            // ── 호출 규약 그대로: 기록 **먼저**, 그 다음 writer 채널 인계 ──
            assert!(rec_ok(record(sock, 42, text, Origin::Send, None)));
            // ★0.14.24 B2′: 프로덕션 send_text 비-human 경로는 이제 `Program`(바이트·flush 는 Data 와
            //   동일 · writer 가 제출 CR 간격 기준점만 추가로 찍는다) — 모델을 프로덕션 변형에 맞춘다.
            tx.send(WriteReq::Program(text.as_bytes().to_vec())).unwrap();
            drop(tx);
            h.join().ok();

            assert!(wrote.load(Ordering::SeqCst), "writer 가 실제로 썼어야 한다");
            assert!(
                seen_in_ledger.load(Ordering::SeqCst),
                "주입 시점에 원장에 레코드가 없었다 — race 봉쇄 실패(훅이 기계 push 를 오너 임무로 기록한다)"
            );
            let _ = Mutex::new(()); // (import 사용 — 하네스 선례와 동일 형태 유지)
        });
    }

    /// 라벨이 **없는** 문안이 원장에 남는가 = 사고 경로(정규식으로는 못 잡던 그것)를 원장이 잡는가.
    #[test]
    fn unlabeled_machine_text_is_recorded() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            for t in [
                "다음 액션 착수",
                "이어서 진행해",
                "[[중첩]] 대괄호 우회",
                "［전각］ 라벨 우회",
            ] {
                assert!(rec_ok(record(sock, 9, t, Origin::Send, None)), "기록 실패: {t}");
            }
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            for t in ["다음 액션 착수", "이어서 진행해", "[[중첩]] 대괄호 우회", "［전각］ 라벨 우회"] {
                assert!(body.contains(&digest(t)), "원장에 없다: {t}");
            }
        });
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ★R6 — 멀티라인 배달의 **제출 단위**가 원장에 남는가 (관통 봉합 · 모듈 머리말 R6)
    //
    // 관통 경로: `WriteReq::Program`(종전 `Data`) 은 원시 바이트라 본문 개행이 그대로 Enter 다 → TUI 가 행
    // 단위로 쪼개 제출 → 각 프롬프트는 전문 레코드의 **진부분**이라 층1 이 전건 미스한다.
    // 아래 테스트들이 "원장이 실제 제출 단위를 안다"를 박제한다.
    // ══════════════════════════════════════════════════════════════════════════

    /// 제출 단위 분해 규칙(순수 함수) — 개행 종류·중복·빈 줄.
    #[test]
    fn submit_units_splits_on_every_newline_flavor() {
        assert_eq!(submit_units("한 줄"), vec!["한 줄".to_string()], "단일 행이면 1건");
        assert_eq!(
            submit_units("첫 줄\n둘째 줄\r\n셋째 줄\r넷째 줄"),
            vec![
                "첫 줄".to_string(),
                "둘째 줄".to_string(),
                "셋째 줄".to_string(),
                "넷째 줄".to_string()
            ],
            "LF·CRLF·CR 모두 제출 경계다(CRLF 의 빈 조각은 탈락)"
        );
        assert_eq!(
            submit_units("A\n\n\n  \nB"),
            vec!["A".to_string(), "B".to_string()],
            "빈 줄·공백 줄은 프롬프트가 될 수 없다"
        );
        assert_eq!(
            submit_units("같은 줄\n같은 줄\n다른 줄"),
            vec!["같은 줄".to_string(), "다른 줄".to_string()],
            "중복은 sha 가 같아 원장에서 무의미하다"
        );
    }

    /// ★관통 재현 차단: 멀티라인 push 의 **각 행**이 원장에서 전문 해시로 대조 가능해야 한다.
    /// 이 어서션이 깨지면 그 행이 단독 제출될 때 층1 이 미스하고, 무라벨이면 층2 도 통과해
    /// 오너 임무로 기록된다(2026-08-01 실사고와 같은 결과).
    #[test]
    fn multiline_delivery_records_each_submitted_line() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "[wakeup] 큐 배달 머리말\n다음 액션 착수\n이어서 T5 잔여를 처리해라\n";
            let report = record_full(sock, 11, text, Origin::Send, None);
            assert!(matches!(report.outcome, Outcome::Recorded));
            assert_eq!(report.parts_written, 3, "행 3개가 전부 남아야 한다");
            assert_eq!(report.parts_dropped, 0);
            assert!(report.parts_failed.is_none());
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            for line in ["다음 액션 착수", "이어서 T5 잔여를 처리해라", "[wakeup] 큐 배달 머리말"] {
                assert!(
                    body.contains(&digest(line)),
                    "행이 원장에 없다 — 이 행이 단독 제출되면 층1 이 미스한다: {line}"
                );
            }
            assert!(body.contains(&digest(text)), "전문 레코드도 그대로 남는다(회귀)");
            let recs: Vec<serde_json::Value> = body
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(recs.len(), 4, "전문 1 + 조각 3");
            assert_eq!(recs[0]["units"], 3, "전문 레코드는 제출 단위 수를 안다");
            assert_eq!(recs[1]["part"], 1);
            assert_eq!(recs[1]["parent"], digest(text), "조각은 부모 배달에 결속된다");
            for r in &recs {
                assert_eq!(r["surface"], "11", "조각도 같은 pane 에 결박된다");
                assert_eq!(r["v"], LEDGER_SCHEMA, "조각도 같은 스키마다(구 판독자 호환)");
                assert_eq!(r["origin"], "send");
            }
        });
    }

    /// 단일 행 배달은 원장 모양이 종전 그대로다(조각 0건) — 원장 예산 회귀 방지.
    #[test]
    fn single_line_delivery_adds_no_parts() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let r = record_full(sock, 5, "  다음 액션 착수  ", Origin::Queue, None);
            assert_eq!(r.parts_written, 0);
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            assert_eq!(body.lines().count(), 1, "전문 1건뿐이어야 한다");
            let v: serde_json::Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
            assert_eq!(v["units"], 1, "쪼개질 수 없는 배달임을 판독자에게 알린다");
        });
    }

    /// 상한 초과는 **조용히 버리지 않는다** — 남긴 수·버린 수가 보고되어야 감사가 성립한다.
    ///
    /// ★R7: 보고 채널이 둘이다. ⓐ`RecordReport`(호출부 → 데몬 버스 이벤트) ⓑ**전문 레코드의
    /// `parts_capped` 필드**(→ 임무 게이트). ⓑ 가 없으면 게이트는 초과를 영영 모른다 —
    /// 데몬 버스는 `javis_mission.py` 의 판독 경로가 아니기 때문이다(SOT §4-7 ⓑ).
    #[test]
    fn part_cap_is_reported_not_silent() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text: String = (0..MAX_PARTS + 7)
                .map(|i| format!("행 번호 {i} 의 지시문\n"))
                .collect();
            let r = record_full(sock, 3, &text, Origin::Send, None);
            assert_eq!(r.parts_written, MAX_PARTS);
            assert_eq!(r.parts_dropped, 7, "버린 조각 수가 사실대로 보고돼야 한다");
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            let head: serde_json::Value =
                serde_json::from_str(body.lines().next().unwrap()).unwrap();
            assert_eq!(
                head["parts_capped"], 7,
                "판독자(임무 게이트)가 보는 자리에 초과 사실이 없다 — 데몬 버스만으로는 \
                 게이트가 영영 모른다(조용한 실패)"
            );
        });
    }

    /// 상한을 넘지 않은 배달에는 `parts_capped` 가 **아예 없다** — 평시 레코드 모양 불변.
    ///
    /// 필드가 상시 붙으면(예: `parts_capped: 0`) 판독자 쪽 fail-closed 조건이 "0 인지 보기"에
    /// 의존하게 되고, 그 비교를 한 번 잘못 쓰는 순간 **모든 배달이 접힌다**(오너 전면 차단 =
    /// 부트스트랩 사망). 존재 자체가 곧 이상이라는 모양이 그 사고를 구조적으로 막는다.
    #[test]
    fn uncapped_delivery_carries_no_capped_marker() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "첫 행 지시\n둘째 행 지시\n셋째 행 지시";
            let r = record_full(sock, 6, text, Origin::Send, None);
            assert_eq!(r.parts_dropped, 0);
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            for line in body.lines() {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                assert!(
                    v.get("parts_capped").is_none(),
                    "상한을 넘지 않았는데 초과 표식이 붙었다: {line}"
                );
            }
        });
    }

    /// ★R7 회귀 핀 ① — **실배포 디렉티브가 조각 상한에 얼마나 다가갔는가**를 파일에서 직접 잰다.
    ///
    /// ## 왜 이 핀이 필요한가 (라운드6 검증자 적발의 본체)
    /// 상한은 숫자로 박혀 있고 디렉티브는 **라운드마다 커진다**. 둘의 거리는 아무도 안 보면
    /// 조용히 0 이 되고, 그 순간부터 초과분 행은 원장에 없다 = 그 행이 단독 제출되면 기계 push 가
    /// 오너 임무를 발급한다. 그래서 거리를 **테스트가 지킨다**.
    ///
    /// ## 무엇을 재는가 — 파일 하나가 아니라 **합성 문안**
    /// pane 에 실제로 들어가는 것은 `cys.rs::compose_directive` 의 출력이다(역할 디렉티브 +
    /// RSI + soul.md + 장기메모리 색인 + 스킬 색인). 라운드6 은 `MASTER_DIRECTIVE.md` **단독**
    /// 454 단위만 보고 "여유 46" 이라고 했지만, 합성본은 master 기준 700 단위대여서 종전 상한
    /// 500 은 **이미 초과 상태**였다. 이 핀은 그 착시를 다시 만들지 않는다.
    /// (합성 로직은 `cys` 바이너리에 있으므로 여기서는 **같은 구성요소를 같은 순서로 이어붙여**
    ///  재현한다. 구성요소가 늘면 이 핀이 과소평가하게 되므로 목록은 compose_directive 와 함께
    ///  갱신한다 — 그래도 '파일 하나만 보던' 종전보다 언제나 정확하다.)
    #[test]
    fn deployed_directive_payload_fits_part_cap_with_headroom() {
        // 상한 대비 이 배수 이상 여유가 없으면 실패한다(=상한을 올리거나 문안을 줄이라는 신호).
        // 4 배: 디렉티브가 지금의 4 배가 되기 전에 반드시 사람이 한 번 본다.
        const REQUIRED_HEADROOM: usize = 4;
        let pack = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack");
        let read = |rel: &str| std::fs::read_to_string(pack.join(rel)).unwrap_or_default();
        // compose_directive 와 같은 순서 — 역할 디렉티브 → RSI → soul → 장기메모리 색인 → 스킬 색인
        let mut composed = read("directives/MASTER_DIRECTIVE.md");
        composed.push_str(&read("directives/RSI_LEARNING_DIRECTIVE.md"));
        composed.push_str(&read("soul.md"));
        composed.push_str(&read("memory/MEMORY.md"));
        if let Ok(entries) = std::fs::read_dir(pack.join("skills")) {
            for e in entries.flatten() {
                let skill = std::fs::read_to_string(e.path().join("SKILL.md")).unwrap_or_default();
                // 색인은 `- <name>: <description>` 한 줄이다(compose_directive 와 동형).
                let field = |k: &str| {
                    skill
                        .lines()
                        .take(10)
                        .find_map(|l| l.strip_prefix(k))
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                let (name, desc) = (field("name:"), field("description:"));
                if !name.is_empty() {
                    composed.push_str(&format!("\n- {name}: {desc}"));
                }
            }
        }
        assert!(
            !composed.is_empty(),
            "합성 문안이 비었다 — 팩 경로가 갈렸다({})",
            pack.display()
        );
        let units = submit_units(&composed).len();
        assert!(
            units * REQUIRED_HEADROOM <= MAX_PARTS,
            "실배포 디렉티브 합성 문안이 {units} 제출단위인데 조각 상한은 {MAX_PARTS} 다 \
             (요구 여유 {REQUIRED_HEADROOM}배 = {} 단위 이하). 상한을 올리거나 문안을 줄여라 \
             — 상한을 넘긴 행은 원장에 없고, 그 행이 단독 제출되면 임무 게이트가 기계 push 를 \
             오너 임무로 오인한다(SOT §4-7 ⓑ).",
            MAX_PARTS / REQUIRED_HEADROOM
        );
    }

    /// ★(W4 · D2 part-cap CEO 변형) — 핀 ① 의 **CEO 승격 합성 케이스**(≈59KB 페이로드 상당).
    ///
    /// 승격 후 CEO pane 에 실제로 들어가는 라이브 md 는 마스터 단독이 아니라 **CEO 합성본**
    /// = [CEO 머리글 fragment] + [합성 서문 ≈600자] + [구분선] + [MASTER 본문 바이트 무수정]
    /// (스펙 §D2 — 기대 수치 ≈780 제출단위 · 4배 여유 3,120 ≤ 4,000). 핀 ① 이 master 합성만
    /// 재면 승격 함대의 실배포 규모가 사각이 된다 — 여기서 그 규모를 직접 잰다.
    ///
    /// ★전방 호환(gen_ceo_template.py 재합성 이전/이후 자기조정): D2 재합성 후에는
    /// `CEO_TEMPLATE.md` 파일 자체가 이미 [머리글+서문+MASTER 연접]이다 — 그때 이 테스트가
    /// MASTER 를 또 이어붙이면 이중 합산(≈113KB)으로 핀이 **오발화**한다. 그래서 템플릿이
    /// MASTER 본문을 이미 포함하면(containment 판별) 파일 그대로를 합성본으로 쓰고,
    /// 현행 6KB 머리글 템플릿이면 스펙 §D2 모양대로 여기서 연접한다.
    #[test]
    fn deployed_ceo_directive_payload_fits_part_cap_with_headroom() {
        const REQUIRED_HEADROOM: usize = 4;
        let pack = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack");
        let read = |rel: &str| std::fs::read_to_string(pack.join(rel)).unwrap_or_default();
        let ceo = read("directives/CEO_TEMPLATE.md");
        let master = read("directives/MASTER_DIRECTIVE.md");
        assert!(
            !ceo.is_empty() && !master.is_empty(),
            "팩 경로가 갈렸다({}) — CEO/MASTER 디렉티브를 읽어야 실측이다",
            pack.display()
        );
        let synthesized = if ceo.contains(master.trim_end()) {
            ceo // 재합성 이후: 파일이 곧 완성 합성본(이중 합산 금지)
        } else {
            // 재합성 이전: §D2 모양(머리글+서문 600자 상당+구분선+MASTER 무수정)을 여기서 연접.
            let preamble: String = std::iter::repeat(
                "직할/부서 위임 판단 트리·자원 관할 분리·RSI 범위·verdict 계약 — CEO 합성 서문 상당 문안\n",
            )
            .take(15)
            .collect();
            format!("{ceo}\n{preamble}\n---\n\n{master}")
        };
        assert!(
            synthesized.len() >= 55_000,
            "합성본이 ≈59KB 페이로드 상당이어야 케이스가 성립한다 (실측 {} bytes)",
            synthesized.len()
        );
        // compose_directive 후첨과 같은 순서 — RSI → soul → 장기메모리 색인 → 스킬 색인(핀 ① 동형).
        let mut composed = synthesized;
        composed.push_str(&read("directives/RSI_LEARNING_DIRECTIVE.md"));
        composed.push_str(&read("soul.md"));
        composed.push_str(&read("memory/MEMORY.md"));
        if let Ok(entries) = std::fs::read_dir(pack.join("skills")) {
            for e in entries.flatten() {
                let skill = std::fs::read_to_string(e.path().join("SKILL.md")).unwrap_or_default();
                let field = |k: &str| {
                    skill
                        .lines()
                        .take(10)
                        .find_map(|l| l.strip_prefix(k))
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                let (name, desc) = (field("name:"), field("description:"));
                if !name.is_empty() {
                    composed.push_str(&format!("\n- {name}: {desc}"));
                }
            }
        }
        let units = submit_units(&composed).len();
        assert!(
            units * REQUIRED_HEADROOM <= MAX_PARTS,
            "CEO 합성 문안이 {units} 제출단위인데 조각 상한은 {MAX_PARTS} 다 \
             (요구 여유 {REQUIRED_HEADROOM}배 = {} 단위 이하 · 스펙 기대 ≈780 단위·3,120≤4,000). \
             상한을 올리거나 서문/디렉티브를 줄여라 — 초과 행은 원장에 없고, 그 행이 단독 \
             제출되면 임무 게이트가 기계 push 를 오너 임무로 오인한다(SOT §4-7 ⓑ).",
            MAX_PARTS / REQUIRED_HEADROOM
        );
    }

    /// ★R7 회귀 핀 ② — **디렉티브급 push 1회의 원장 비용과 회전까지의 횟수**를 실측으로 묶는다.
    ///
    /// R6 이 조각 레코드를 도입하며 회전이 572 배 빨라졌다는 것이 라운드6 검증자의 실측이었다
    /// (회전으로 소실된 구간은 층1 대조가 불가능 = SOT §4-6 ⓑ '치명 방향'). 여기서 재는 것은
    /// **바이트/1회 push** 와 **`LEDGER_MAX_BYTES` 기준 회전까지 push 횟수**이며, 수치가 나빠지면
    /// (레코드가 다시 비대해지거나 상한이 내려가면) 실패한다.
    #[test]
    fn directive_grade_push_keeps_rotation_budget() {
        // 디렉티브급 push 를 이 횟수 이상 담지 못하면 실패(=회전 꼬리가 다시 짧아졌다).
        const REQUIRED_PUSHES_TO_ROTATE: u64 = 30;
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            // 실배포 디렉티브 본문을 그대로 쓴다(합성 규모 = 700 단위대의 대표값).
            let pack = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack");
            let mut text = std::fs::read_to_string(pack.join("directives/MASTER_DIRECTIVE.md"))
                .expect("배포 디렉티브를 읽어야 실측이다");
            text.push_str(
                &std::fs::read_to_string(pack.join("directives/RSI_LEARNING_DIRECTIVE.md"))
                    .unwrap_or_default(),
            );
            let units = submit_units(&text).len();
            let r = record_full(sock, 77, &text, Origin::Send, None);
            assert_eq!(r.parts_dropped, 0, "이 규모는 상한 안이어야 한다(핀 ①과 같은 취지)");
            let bytes = std::fs::metadata(ledger_path(sock)).unwrap().len();
            let pushes = LEDGER_MAX_BYTES / bytes.max(1);
            assert!(
                pushes >= REQUIRED_PUSHES_TO_ROTATE,
                "디렉티브급 push({units} 단위)가 1회에 {bytes} B 를 쓴다 — 원장 상한 \
                 {LEDGER_MAX_BYTES} B 기준 {pushes} 회면 회전한다(요구 {REQUIRED_PUSHES_TO_ROTATE} \
                 회 이상). 회전으로 소실된 구간은 층1 대조가 불가능하다(SOT §4-6 ⓑ)."
            );
            // 조각 1건당 바이트도 함께 고정한다(레코드가 다시 비대해지는 회귀 방지).
            let per_part = bytes / (r.parts_written as u64 + 1);
            assert!(
                per_part <= 400,
                "조각 1건이 평균 {per_part} B 다 — 경량화(ts/from 제거·preview {PART_PREVIEW_CHARS}자)가 \
                 풀렸는지 확인하라"
            );
        });
    }

    /// ★R7 회귀 핀 ③ — **생산자 상한과 판독자 상한이 함께 움직였는가**(교차언어 결합).
    ///
    /// `LEDGER_MAX_BYTES` 를 올리면서 판독자(`javis_mission.py`)의
    /// `LEDGER_MAX_READ_BYTES`·`DELIVERY_SCAN_LINES` 를 그대로 두면 **정상 데몬 출력이 판독자의
    /// '조작 정황' 상한을 넘긴다** → 원장이 판독 불가로 접히고, 그 상태에서는 오너가 임무를 줄
    /// 수 없다(부트스트랩 불가침 위반 · 차단이 만든 가용성 사고). 두 파일이 다른 언어·다른
    /// 배포 단위라 사람이 한쪽만 고치기 쉬운 자리이므로, 결합을 테스트로 못 박는다.
    #[test]
    fn reader_scan_limits_track_producer_rotation_cap() {
        let py = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack/bin/javis_mission.py");
        let src = std::fs::read_to_string(&py).expect("판독자 소스를 읽어야 결합을 확인한다");
        // `NAME = <식>` 한 줄에서 정수 리터럴 곱을 계산한다(주석은 `#` 이후를 버린다).
        let konst = |name: &str| -> u64 {
            let line = src
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("{name} = ")))
                .unwrap_or_else(|| panic!("판독자에서 {name} 을 못 찾았다 — 이름이 갈렸다"));
            let expr = line.split('=').nth(1).unwrap().split('#').next().unwrap();
            expr.split('*')
                .map(|t| t.trim().parse::<u64>().expect("정수 리터럴 곱만 지원한다"))
                .product()
        };
        // 상한 초과 표식의 **필드명**이 양쪽에서 같아야 한다. 이름이 갈리면 데몬은 성실히
        // 남기고 판독자는 영영 못 보는 상태가 되며, 그건 R7 이전(흔적 0)과 정확히 같다.
        assert!(
            src.contains("\"parts_capped\""),
            "판독자가 `parts_capped` 필드를 읽지 않는다 — 생산자만 남기면 임무 게이트는 상한 \
             초과를 영영 모른다(조용한 실패)"
        );
        let read_cap = konst("LEDGER_MAX_READ_BYTES");
        let scan_cap = konst("DELIVERY_SCAN_LINES");
        assert!(
            read_cap >= LEDGER_MAX_BYTES,
            "판독자 세대당 바이트 상한({read_cap})이 데몬 회전 상한({LEDGER_MAX_BYTES})보다 작다 \
             — 정상 원장이 '판독 불가'가 되어 게이트가 영구 잠긴다"
        );
        // 데몬이 만들 수 있는 최대 줄 수(2세대 · 회전 검사는 append 앞이므로 세대당 상한 + push 1회).
        // 최소 레코드 바이트는 보수적으로 잡는다(작게 잡을수록 요구 줄수가 커진다 = 안전측).
        const MIN_RECORD_BYTES: u64 = 150;
        let max_push_bytes = MAX_PARTS as u64 * 400;
        let producible_lines = 2 * (LEDGER_MAX_BYTES + max_push_bytes) / MIN_RECORD_BYTES;
        assert!(
            scan_cap > producible_lines,
            "판독자 스캔 상한({scan_cap} 줄)이 데몬이 정상 동작으로 만들 수 있는 줄 수\
             ({producible_lines})보다 작다 — 평시 원장이 '조작 정황'으로 접힌다(fail-closed 오발)"
        );
    }

    /// 조각 레코드는 **판독자 필수 필드**를 전부 갖고, 뺀 필드(`ts`·`from`)는 전문에만 있다.
    ///
    /// 경량화가 판독자 계약을 건드리면 층1 이 **조용히** 죽는다(양쪽 테스트가 다 초록인 채로).
    /// 그래서 뺄 수 있는 것과 없는 것을 여기서 못 박는다.
    #[test]
    fn part_records_are_slim_but_keep_reader_contract() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "첫 행 지시문입니다\n둘째 행 지시문입니다";
            let _ = record_full(sock, 9, text, Origin::Send, Some(3));
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            let recs: Vec<serde_json::Value> = body
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(recs.len(), 3, "전문 1 + 조각 2");
            // 전문 레코드는 종전 그대로(감사 머리)
            assert!(!recs[0]["ts"].is_null() && !recs[0]["from"].is_null());
            for r in &recs[1..] {
                for k in ["v", "surface", "ts_epoch", "sha256", "chars", "preview", "origin"] {
                    assert!(!r[k].is_null(), "판독자 필수 필드 {k} 가 조각에서 사라졌다: {r}");
                }
                assert_eq!(r["parent"], recs[0]["sha256"], "감사 결속(parent)은 유지한다");
                assert_eq!(r["ts_epoch"], recs[0]["ts_epoch"], "같은 배달임이 ts_epoch 로 결속된다");
                assert!(r.get("ts").is_none(), "조각의 ISO ts 는 뺀다(ts_epoch 로 유도)");
                assert!(r.get("from").is_none(), "조각의 from 은 뺀다(전문에 있다)");
            }
        });
    }

    /// ★교차언어 앵커(R6) — python 판독자가 **진짜 생산자 출력**으로 접히는지 확인할 수단.
    ///
    /// 왜 필요한가: `javis_mission.py` self-test 는 원장 fixture 를 **손으로 미러**한다
    /// (`_rec_multiline`). 미러가 생산자와 갈리면 양쪽 테스트가 다 초록인 채로 층1 이 조용히
    /// 무력화된다 — 이 모듈이 정규화·해시를 양쪽에 박제한 것과 같은 위험이다. 그래서 진짜
    /// `record_full` 출력을 파일로 떨어뜨려 python 이 그대로 읽게 한다.
    ///
    /// 평시엔 아무 것도 내보내지 않는다. `CYS_XLANG_LEDGER_OUT=<dir>` 이 있을 때만 그 디렉터리에
    /// 레인 규약 파일명(`delivery-base.jsonl`)으로 복사한다 — 소비 절차:
    ///   `CYS_XLANG_LEDGER_OUT=/tmp/x cargo test --bin cysd xlang_ledger_fixture`
    ///   `CYS_STATE_DIR=/tmp/x CYS_SURFACE_ID=4242 python3 …/javis_mission.py delivery-path --json`
    #[test]
    fn xlang_ledger_fixture_matches_reader_expectations() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "[wakeup] 자동 기상 알림\n다음 액션 착수\n이어서 T5 잔여 항목을 처리하고 결과를 보고하라\n";
            let r = record_full(sock, 4242, text, Origin::Send, None);
            assert_eq!(r.parts_written, 3);
            let src = ledger_path(sock);
            // 판독자가 **반드시** 보는 필드들(하나라도 이름이 바뀌면 층1 이 조용히 죽는다)
            let body = std::fs::read_to_string(&src).unwrap();
            for line in body.lines() {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                for k in ["v", "surface", "ts_epoch", "sha256", "chars", "preview", "origin"] {
                    assert!(!v[k].is_null(), "판독자 필수 필드 {k} 가 없다: {line}");
                }
            }
            if let Ok(out) = std::env::var("CYS_XLANG_LEDGER_OUT") {
                if !out.trim().is_empty() {
                    let dir = PathBuf::from(out);
                    std::fs::create_dir_all(&dir).expect("교차언어 출력 디렉터리");
                    std::fs::copy(&src, dir.join("delivery-base.jsonl")).expect("fixture 복사");
                }
            }
        });
    }

    /// 기록은 **주입 직전**이므로 조각까지 포함해 flush 가 끝나 있어야 한다(불변식 ①의 확장).
    #[test]
    fn parts_are_flushed_before_record_returns() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "첫 행 지시\n둘째 행 지시";
            let _ = record_full(sock, 8, text, Origin::Schedule, None);
            // 반환 직후 **다른 판독자**(훅)가 읽는 상황을 그대로 재현한다.
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            assert!(body.contains(&digest("둘째 행 지시")), "반환 시점에 조각이 디스크에 없다");
        });
    }

    #[test]
    fn epoch_marker_is_atomic_and_parseable() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            write_epoch(sock);
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(epoch_path(sock)).unwrap()).unwrap();
            assert!(v["daemon_epoch"].as_f64().unwrap() > 0.0);
            assert_eq!(v["v"], LEDGER_SCHEMA);
            assert!(!epoch_path(sock).with_extension("json.tmp").exists(), "tmp 잔재 없음");
        });
    }
}
